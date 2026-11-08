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
                if let Some(Part::Field(key)) = child.last()
                    && let Some(value) = expand(&child, cells)
                {
                    fields.insert(key.clone(), value);
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
                if seen.insert(child.clone())
                    && let Some(value) = expand(&child, cells)
                {
                    items.push(value);
                }
            }
            // Concurrent additions absent from the winning order still exist.
            for child in children {
                if seen.insert(child.clone())
                    && let Some(value) = expand(&child, cells)
                {
                    items.push(value);
                }
            }
            Some(Value::Array(items))
        }
    }
}
/// A path's direct children. Paths sharing a prefix sort together, so this
/// walks only the subtree instead of every cell of the document.
fn children(path: &Path, cells: &Cells) -> BTreeSet<Path> {
    cells
        .range::<Path, _>((std::ops::Bound::Excluded(path), std::ops::Bound::Unbounded))
        .map(|(p, _)| p)
        .take_while(|p| p.starts_with(path))
        .map(|p| p[..path.len() + 1].to_vec())
        .collect()
}

/// One save of a shared document, or a snapshot standing in for many.
///
/// The snapshot fields are skipped when empty, so an ordinary edit serializes
/// exactly as older builds wrote it, and older builds (which ignore unknown
/// fields) read a snapshot as an ordinary edit that sets the whole document.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Edit {
    pub id: String,
    pub document: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub clock: i64,
    /// This edit is a snapshot: `after` is the whole document as of the edits
    /// and snapshots in `folds`, which it replaces.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub snapshot: bool,
    /// For a snapshot, the IDs it replaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<String>,
}

impl Edit {
    fn order(&self) -> (i64, &str) {
        (self.clock, &self.id)
    }
}

/// How one document's edits are read: the snapshot it starts from, and the
/// IDs that snapshot already accounts for.
struct Lineage<'a> {
    primary: Option<&'a Edit>,
    /// Everything the primary snapshot replaced, directly or through the
    /// snapshots it replaced. Copies of these still around are stale.
    covered: BTreeSet<&'a str>,
}

/// Works out a document's [`Lineage`] from its snapshots.
///
/// A head is a snapshot no other present snapshot folds. Normally there is one.
/// Two devices compacting at the same moment can leave two; the latest by
/// `(clock, id)` is the primary, and the others are ignored. What only a
/// losing snapshot folded is then not covered, so those edits (which are
/// still present: see `GRACE` in the sync worker) replay as ordinary edits and
/// nothing is lost. The next snapshot folds the losers away.
fn lineage<'a>(snapshots: &[&'a Edit]) -> Lineage<'a> {
    let folded: BTreeSet<&str> = snapshots
        .iter()
        .flat_map(|s| s.folds.iter().map(String::as_str))
        .collect();
    let primary = snapshots
        .iter()
        .filter(|s| !folded.contains(s.id.as_str()))
        .max_by(|a, b| a.order().cmp(&b.order()))
        .copied();
    let mut covered = BTreeSet::new();
    let mut frontier: Vec<&Edit> = primary.into_iter().collect();
    while let Some(snapshot) = frontier.pop() {
        for id in &snapshot.folds {
            if covered.insert(id.as_str())
                && let Some(inner) = snapshots.iter().find(|s| s.id == *id)
            {
                frontier.push(inner);
            }
        }
    }
    Lineage { primary, covered }
}

/// The order edits are replayed in, and whether each is read whole.
///
/// Without snapshots this is every edit by `(clock, id)`. With them, each
/// document starts from its primary snapshot (see [`lineage`]) read as the
/// whole document, followed by every edit it does not cover by `(clock, id)`.
/// That includes an edit made offline before the snapshot that the snapshot
/// never saw: it is applied on top rather than lost under it.
///
/// Folding the oldest edits into a snapshot therefore leaves the projection
/// unchanged, which is what lets the journal be compacted.
fn replay(edits: &[Edit]) -> Vec<(&Edit, bool)> {
    let mut by_document = BTreeMap::<&str, (Vec<&Edit>, Vec<&Edit>)>::new();
    for edit in edits {
        let (snapshots, rest) = by_document.entry(edit.document.as_str()).or_default();
        if edit.snapshot {
            snapshots.push(edit);
        } else {
            rest.push(edit);
        }
    }
    let mut order = Vec::new();
    for (_, (snapshots, mut rest)) in by_document {
        let lineage = lineage(&snapshots);
        rest.retain(|edit| !lineage.covered.contains(edit.id.as_str()));
        rest.sort_by(|a, b| a.order().cmp(&b.order()));
        order.extend(lineage.primary.map(|primary| (primary, true)));
        order.extend(rest.into_iter().map(|edit| (edit, false)));
    }
    order
}

/// The IDs of `document`'s edits and snapshots that its primary snapshot
/// accounts for, and that can be discarded once it is settled.
pub fn covered(edits: &[Edit], document: &str) -> Vec<String> {
    let snapshots: Vec<&Edit> = edits
        .iter()
        .filter(|e| e.snapshot && e.document == document)
        .collect();
    let lineage = lineage(&snapshots);
    edits
        .iter()
        .filter(|e| e.document == document && lineage.covered.contains(e.id.as_str()))
        .map(|e| e.id.clone())
        .collect()
}

/// The snapshots of `document` that no other present snapshot folds.
pub fn heads<'a>(edits: &'a [Edit], document: &str) -> Vec<&'a Edit> {
    let snapshots: Vec<&Edit> = edits
        .iter()
        .filter(|e| e.snapshot && e.document == document)
        .collect();
    let folded: BTreeSet<&str> = snapshots
        .iter()
        .flat_map(|s| s.folds.iter().map(String::as_str))
        .collect();
    snapshots
        .into_iter()
        .filter(|s| !folded.contains(s.id.as_str()))
        .collect()
}

/// Invalid/future JSON is an error, never an empty document. Superseded edits
/// stay in the journal until they are folded into a snapshot.
pub fn project(edits: &[Edit]) -> Result<BTreeMap<String, Option<String>>, serde_json::Error> {
    let mut documents = BTreeMap::<String, Cells>::new();
    for (edit, whole) in replay(edits) {
        let mut before = Cells::new();
        let mut after = Cells::new();
        // The snapshot a document starts from is the whole document.
        let before_text = if whole { &None } else { &edit.before };
        if let Some(text) = before_text {
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

/// Plans a snapshot of `document` that folds its oldest edits, leaving the
/// newest `keep` as they are. Only a run of edits from the oldest on that all
/// satisfy `foldable` is folded, so an edit not yet on the server is never
/// replaced by a snapshot of it.
///
/// The snapshot also folds every snapshot of the document already present,
/// so it is the only head afterwards. Its `id` is left empty for the caller.
/// Returns `None` when nothing would be folded.
pub fn plan_snapshot(
    edits: &[Edit],
    document: &str,
    keep: usize,
    foldable: impl Fn(&Edit) -> bool,
) -> Result<Option<Edit>, serde_json::Error> {
    let order: Vec<&Edit> = replay(edits)
        .into_iter()
        .map(|(edit, _)| edit)
        .filter(|edit| edit.document == document && !edit.snapshot)
        .collect();
    let Some(excess) = order.len().checked_sub(keep).filter(|n| *n > 0) else {
        return Ok(None);
    };
    let folded: Vec<&Edit> = order
        .into_iter()
        .take(excess)
        .take_while(|edit| foldable(edit))
        .collect();
    if folded.is_empty() {
        return Ok(None);
    }
    let snapshots: Vec<&Edit> = edits
        .iter()
        .filter(|e| e.snapshot && e.document == document)
        .collect();
    let mut together: Vec<Edit> = snapshots.iter().map(|e| (*e).clone()).collect();
    together.extend(folded.iter().map(|e| (*e).clone()));
    let after = project(&together)?.remove(document).flatten();
    Ok(Some(Edit {
        id: String::new(),
        document: document.to_owned(),
        before: None,
        after,
        clock: together.iter().map(|e| e.clock).max().unwrap_or(1),
        snapshot: true,
        folds: together.iter().map(|e| e.id.clone()).collect(),
    }))
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
            ..Default::default()
        }
    }

    /// A chain of edits that each change one field, as saves from the app do.
    fn history(n: i64) -> Vec<Edit> {
        use serde_json::json;
        let mut edits = Vec::new();
        let mut doc = json!({"habits":[{"id":"a","marks":{}},{"id":"b","name":"B"}]});
        edits.push(edit("e0", 1, None, Some(doc.clone())));
        for i in 1..n {
            let before = doc.clone();
            doc["habits"][0]["marks"][format!("day{i}")] = json!(i);
            edits.push(edit(
                &format!("e{i}"),
                i + 1,
                Some(before),
                Some(doc.clone()),
            ));
        }
        edits
    }

    /// Folds `edits` as the sync worker would once the snapshot is settled.
    fn compact(edits: &[Edit], id: &str, keep: usize) -> (Edit, Vec<Edit>) {
        let mut snapshot = plan_snapshot(edits, "habits", keep, |_| true)
            .unwrap()
            .unwrap();
        snapshot.id = id.into();
        let mut with = edits.to_vec();
        with.push(snapshot.clone());
        let gone = covered(&with, "habits");
        with.retain(|e| !gone.contains(&e.id));
        (snapshot, with)
    }

    #[test]
    fn folding_the_oldest_edits_leaves_the_projection_unchanged() {
        let edits = history(30);
        let full = value(&edits);
        let (snapshot, remaining) = compact(&edits, "s1", 5);
        assert_eq!(snapshot.folds.len(), 25);
        assert_eq!(remaining.len(), 6);
        assert_eq!(value(&remaining), full);

        // Compacting again folds the previous snapshot too, leaving one.
        let mut more = remaining.clone();
        let last = value(&remaining);
        let mut next = last.clone();
        next["habits"][1]["name"] = serde_json::json!("Renamed");
        for i in 0..6 {
            more.push(edit(
                &format!("f{i}"),
                100 + i,
                Some(last.clone()),
                Some(next.clone()),
            ));
        }
        let expected = value(&more);
        let (second, left) = compact(&more, "s2", 3);
        assert!(second.folds.contains(&"s1".to_owned()));
        assert_eq!(left.iter().filter(|e| e.snapshot).count(), 1);
        assert_eq!(left.len(), 4);
        assert_eq!(value(&left), expected);
    }

    #[test]
    fn stale_copies_of_folded_edits_are_ignored() {
        let edits = history(10);
        let full = value(&edits);
        let (first, _) = compact(&edits, "s1", 2);
        let mut everything = edits.clone();
        everything.push(first.clone());
        // A device that has not yet discarded the folded edits still agrees,
        assert_eq!(value(&everything), full);
        // including edits two generations old.
        let mut second = plan_snapshot(&everything, "habits", 0, |_| true)
            .unwrap()
            .unwrap();
        second.id = "s2".into();
        assert_eq!(second.folds.len(), 3);
        everything.push(second);
        assert_eq!(value(&everything), full);
    }

    #[test]
    fn an_offline_edit_older_than_the_snapshot_is_applied_on_top_of_it() {
        use serde_json::json;
        let edits = history(10);
        let seed = serde_json::from_str::<Value>(edits[0].after.as_ref().unwrap()).unwrap();
        let (_, mut remaining) = compact(&edits, "s1", 2);
        let mut renamed = seed.clone();
        renamed["habits"][1]["name"] = json!("Offline");
        // Clock 2: made long ago on a device that never saw the later edits.
        remaining.push(edit("offline", 2, Some(seed), Some(renamed)));
        let result = value(&remaining);
        assert_eq!(result["habits"][1]["name"], json!("Offline"));
        assert_eq!(result["habits"][0]["marks"]["day9"], json!(9));
    }

    #[test]
    fn a_losing_concurrent_snapshot_loses_nothing() {
        use serde_json::json;
        let edits = history(4);
        let base = value(&edits);
        let mut a = base.clone();
        a["habits"][0]["marks"]["from_a"] = json!(1);
        let mut b = base.clone();
        b["habits"][1]["name"] = json!("From B");
        let a1 = edit("a1", 10, Some(base.clone()), Some(a));
        let b1 = edit("b1", 11, Some(base.clone()), Some(b));
        // Each device folds everything it has, before seeing the other's edit.
        let mut on_a = edits.clone();
        on_a.push(a1.clone());
        let mut on_b = edits.clone();
        on_b.push(b1.clone());
        let mut snap_a = plan_snapshot(&on_a, "habits", 0, |_| true)
            .unwrap()
            .unwrap();
        snap_a.id = "snap-a".into();
        let mut snap_b = plan_snapshot(&on_b, "habits", 0, |_| true)
            .unwrap()
            .unwrap();
        snap_b.id = "snap-b".into();
        // Within the grace period the folded edits are all still present.
        let mut all = edits.clone();
        all.extend([a1, b1, snap_a.clone(), snap_b.clone()]);
        let merged = value(&all);
        assert_eq!(merged["habits"][0]["marks"]["from_a"], json!(1));
        assert_eq!(merged["habits"][1]["name"], json!("From B"));
        assert_eq!(heads(&all, "habits").len(), 2);
        // Every device agrees, whatever order it received things in.
        all.reverse();
        assert_eq!(value(&all), merged);
        // Only what the winner folded is discarded; a1 stays live.
        let gone = covered(&all, "habits");
        assert!(gone.contains(&"b1".to_owned()));
        assert!(!gone.contains(&"a1".to_owned()));
        // The next snapshot folds both heads, leaving one.
        let next = plan_snapshot(&all, "habits", 0, |_| true).unwrap().unwrap();
        assert!(next.folds.contains(&"snap-a".to_owned()));
        assert!(next.folds.contains(&"snap-b".to_owned()));
    }

    #[test]
    fn unfoldable_edits_stop_the_fold() {
        let edits = history(10);
        let plan = plan_snapshot(&edits, "habits", 2, |e| e.id != "e3")
            .unwrap()
            .unwrap();
        assert_eq!(plan.folds, vec!["e0", "e1", "e2"]);
        assert!(
            plan_snapshot(&edits, "habits", 2, |e| e.id != "e0")
                .unwrap()
                .is_none()
        );
        assert!(
            plan_snapshot(&edits, "habits", 10, |_| true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_snapshot_reads_as_an_ordinary_edit_to_older_builds() {
        #[derive(Deserialize)]
        struct Old {
            id: String,
            before: Option<String>,
            after: Option<String>,
            clock: i64,
        }
        let (snapshot, _) = compact(&history(5), "s1", 1);
        let old: Old = serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(old.id, "s1");
        assert!(old.before.is_none());
        assert_eq!(old.after, snapshot.after);
        assert_eq!(old.clock, snapshot.clock);
        // And an ordinary edit is written exactly as before.
        let plain = serde_json::to_value(edit("x", 1, None, None)).unwrap();
        assert_eq!(
            plain.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["after", "before", "clock", "document", "id"]
        );
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
