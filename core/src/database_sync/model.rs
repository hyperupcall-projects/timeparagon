//! Immutable edits are replicated; this deterministic projection merges changes
//! at field boundaries instead of replacing entire JSON collections.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Part {
    Field(String),
    Item(String),
}
type Path = Vec<Part>;
#[derive(Clone, Debug, PartialEq)]
enum Cell {
    Object,
    Array(Vec<String>),
    Scalar(Value),
}
type Cells = BTreeMap<Path, Cell>;

fn identity(value: &Value) -> String {
    if let Some(object) = value.as_object() {
        for field in ["drive_id", "id", "started_at", "recorded_at", "date"] {
            if let Some(value) = object.get(field).filter(|v| !v.is_null()) {
                return format!("{field}:{value}");
            }
        }
    }
    // Lists without stable IDs keep both independently edited variants. Never
    // infer identity from a title, nor silently deduplicate equal occurrences.
    format!("hash:{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn flatten(value: &Value, path: Path, cells: &mut Cells) {
    match value {
        Value::Object(fields) => {
            cells.insert(path.clone(), Cell::Object);
            for (key, value) in fields {
                let mut child = path.clone();
                child.push(Part::Field(key.clone()));
                flatten(value, child, cells);
            }
        }
        Value::Array(items) => {
            let mut occurrences = BTreeMap::<String, usize>::new();
            let mut order = Vec::new();
            for item in items {
                let id = identity(item);
                let occurrence = occurrences.entry(id.clone()).or_default();
                let id = format!("{id}#{occurrence}");
                *occurrence += 1;
                order.push(id.clone());
                let mut child = path.clone();
                child.push(Part::Item(id));
                flatten(item, child, cells);
            }
            cells.insert(path, Cell::Array(order));
        }
        _ => {
            cells.insert(path, Cell::Scalar(value.clone()));
        }
    }
}
fn expand(path: &Path, cells: &Cells) -> Option<Value> {
    match cells.get(path)? {
        Cell::Scalar(value) => Some(value.clone()),
        Cell::Object => {
            let mut fields = serde_json::Map::new();
            for child in children(path, cells) {
                if let Some(Part::Field(key)) = child.last() {
                    if let Some(value) = expand(&child, cells) {
                        fields.insert(key.clone(), value);
                    }
                }
            }
            Some(Value::Object(fields))
        }
        Cell::Array(order) => {
            let children = children(path, cells);
            let mut items = Vec::new();
            let mut seen = BTreeSet::new();
            for id in order {
                let mut child = path.clone();
                child.push(Part::Item(id.clone()));
                if seen.insert(child.clone()) {
                    if let Some(value) = expand(&child, cells) {
                        items.push(value);
                    }
                }
            }
            // Concurrent additions absent from the winning order still exist.
            for child in children {
                if seen.insert(child.clone()) {
                    if let Some(value) = expand(&child, cells) {
                        items.push(value);
                    }
                }
            }
            Some(Value::Array(items))
        }
    }
}
fn children(path: &Path, cells: &Cells) -> BTreeSet<Path> {
    cells
        .keys()
        .filter(|p| p.len() > path.len() && p.starts_with(path))
        .map(|p| p[..path.len() + 1].to_vec())
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edit {
    pub id: String,
    pub document: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub clock: i64,
}

/// Invalid/future JSON is an error, never an empty document. All raw edits
/// remain in the journal even when their fields are superseded.
pub fn project(edits: &[Edit]) -> Result<BTreeMap<String, Option<String>>, serde_json::Error> {
    let mut sorted: Vec<_> = edits.iter().collect();
    sorted.sort_by(|a, b| (a.clock, &a.id).cmp(&(b.clock, &b.id)));
    let mut documents = BTreeMap::<String, Cells>::new();
    for edit in sorted {
        let mut before = Cells::new();
        let mut after = Cells::new();
        if let Some(text) = &edit.before {
            flatten(
                &serde_json::from_str::<Value>(text)?,
                Vec::new(),
                &mut before,
            );
        }
        if let Some(text) = &edit.after {
            flatten(
                &serde_json::from_str::<Value>(text)?,
                Vec::new(),
                &mut after,
            );
        }
        let target = documents.entry(edit.document.clone()).or_default();
        // Parents sort before descendants. A tombstone removes even fields
        // added concurrently, so deleting an entity cannot leave a broken shell.
        for path in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            if before.get(path) == after.get(path) {
                continue;
            }
            match after.get(path) {
                None => target.retain(|p, _| !p.starts_with(path)),
                Some(value) => {
                    if matches!(value, Cell::Scalar(_)) {
                        target.retain(|p, _| !p.starts_with(path));
                    }
                    target.insert(path.clone(), value.clone());
                }
            }
        }
    }
    Ok(documents
        .into_iter()
        .map(|(key, cells)| (key, expand(&Vec::new(), &cells).map(|v| v.to_string())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edit(id: &str, clock: i64, before: Option<Value>, after: Option<Value>) -> Edit {
        Edit {
            id: id.into(),
            clock,
            document: "habits".into(),
            before: before.map(|v| v.to_string()),
            after: after.map(|v| v.to_string()),
        }
    }
    fn value(edits: &[Edit]) -> Value {
        serde_json::from_str(project(edits).unwrap()["habits"].as_ref().unwrap()).unwrap()
    }
    #[test]
    fn offline_edits_to_different_habits_and_days_merge() {
        use serde_json::json;
        let initial =
            json!({"habits":[{"id":"a","name":"A","marks":{}},{"id":"b","name":"B","marks":{}}]});
        let mut a = initial.clone();
        a["habits"][0]["marks"]["2026-11-05"] = json!(1);
        let mut b = initial.clone();
        b["habits"][1]["name"] = json!("Renamed");
        let edits = vec![
            edit("seed", 1, None, Some(initial.clone())),
            edit("a", 2, Some(initial.clone()), Some(a)),
            edit("b", 2, Some(initial), Some(b)),
        ];
        let result = value(&edits);
        assert_eq!(result["habits"][0]["marks"]["2026-11-05"], json!(1));
        assert_eq!(result["habits"][1]["name"], json!("Renamed"));
        let mut reversed = edits.clone();
        reversed.reverse();
        assert_eq!(value(&reversed), result);
    }
    #[test]
    fn concurrent_additions_survive_and_duplicate_occurrences_are_kept() {
        use serde_json::json;
        let empty = json!({"items":[]});
        let edits = vec![
            edit("s", 1, None, Some(empty.clone())),
            edit(
                "a",
                2,
                Some(empty.clone()),
                Some(json!({"items":[{"text":"A"},{"text":"A"}]})),
            ),
            edit("b", 2, Some(empty), Some(json!({"items":[{"text":"B"}]}))),
        ];
        assert_eq!(value(&edits)["items"].as_array().unwrap().len(), 3);
    }
    #[test]
    fn deletion_and_recreation_are_distinct_from_empty_and_null() {
        use serde_json::json;
        let initial = json!({"a":null,"b":[]});
        let mut edits = vec![
            edit("s", 1, None, Some(initial.clone())),
            edit("d", 2, Some(initial.clone()), None),
        ];
        assert_eq!(project(&edits).unwrap()["habits"], None);
        edits.push(edit("r", 3, None, Some(initial.clone())));
        assert_eq!(value(&edits), initial);
    }
    #[test]
    fn unknown_fields_survive_and_bad_json_is_never_replaced() {
        use serde_json::json;
        let initial = json!({"future":{"data":7},"known":1});
        let mut changed = initial.clone();
        changed["known"] = json!(2);
        assert_eq!(
            value(&[
                edit("s", 1, None, Some(initial.clone())),
                edit("a", 2, Some(initial), Some(changed.clone()))
            ]),
            changed
        );
        let mut broken = edit("bad", 1, None, None);
        broken.after = Some("not json".into());
        assert!(project(&[broken]).is_err());
    }
}
