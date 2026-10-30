//! Read-only Google Keep snapshots, obtained through the shared browser helper.
use base64::Engine;
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod cache;
mod metadata;
pub use metadata::Metadata;

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Snapshot {
    pub notes: Vec<Note>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
    pub labels: Vec<String>,
    pub color: String,
    pub preview: Option<String>,
    #[serde(skip)]
    pub preview_handle: Option<iced::widget::image::Handle>,
    pub pinned: bool,
    pub archived: bool,
    pub checklist: Vec<Item>,
    pub metadata: Vec<String>,
    pub details: Metadata,
    pub body: Vec<TextRun>,
}
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Note {
    pub fn parsed_metadata(&self) -> Metadata {
        let mut details = self.details.clone();
        details.parse(&self.metadata, &self.labels);
        details
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Item {
    pub text: String,
    pub checked: bool,
    pub level: usize,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Active,
    All,
    Archived,
}
#[derive(Default)]
pub struct Store {
    pub snapshot: Snapshot,
    pub loading: bool,
    pub error: Option<String>,
    pub query: String,
    pub label: Option<String>,
    pub scope: Scope,
    cache_path: Option<PathBuf>,
}
impl Store {
    /// Only real app startup opts into disk access; test stores stay isolated.
    pub fn cache_in(&mut self, path: Option<PathBuf>) {
        self.cache_path = path;
        if let Some(snapshot) = self.cache_path.as_deref().and_then(cache::load) {
            self.snapshot = snapshot;
        }
    }

    pub fn cache_path(&self) -> Option<PathBuf> {
        self.cache_path.clone()
    }

    pub fn visible(&self) -> Vec<&Note> {
        let mut notes: Vec<_> = self
            .snapshot
            .notes
            .iter()
            .filter_map(|note| {
                if (self.scope == Scope::Active && note.archived)
                    || (self.scope == Scope::Archived && !note.archived)
                    || self.label.as_ref().is_some_and(|label| {
                        !note
                            .labels
                            .iter()
                            .any(|l| l.to_lowercase() == label.to_lowercase())
                    })
                {
                    return None;
                }
                score(note, &self.query).map(|score| (score, note))
            })
            .collect();
        notes.sort_by_key(|(score, note)| (*score, !note.pinned));
        notes.into_iter().map(|(_, note)| note).collect()
    }
    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<_> = self
            .snapshot
            .notes
            .iter()
            .flat_map(|n| n.labels.clone())
            .collect();
        labels.sort_by_key(|l| l.to_lowercase());
        labels.dedup();
        labels
    }
}
fn score(note: &Note, query: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let title = note.title.to_lowercase();
    let body = format!(
        "{} {}",
        note.description,
        note.checklist
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();
    if title.contains(&query) {
        return Some(0);
    }
    if body.contains(&query) {
        return Some(1);
    }
    // Fuzzy match each query word against a complete word. Short queries must
    // match literally; longer words tolerate one or two edits, including swaps.
    let mut total = 2;
    for token in query.split_whitespace() {
        let limit = match token.chars().count() {
            0..=3 => 0,
            4..=7 => 1,
            _ => 2,
        };
        let distance = title
            .split(|c: char| !c.is_alphanumeric())
            .chain(body.split(|c: char| !c.is_alphanumeric()))
            .filter(|w| !w.is_empty() && w.chars().count().abs_diff(token.chars().count()) <= limit)
            .map(|w| edit_distance(token, w))
            .min()?;
        if distance > limit {
            return None;
        }
        total += distance;
    }
    Some(total)
}
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    let mut d = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}
pub fn fetch(cache_path: Option<PathBuf>) -> Result<Snapshot, String> {
    let bytes = run("keep-fetch")?;
    cache::accept(&bytes, cache_path.as_deref())
}

fn decode(bytes: &[u8]) -> Result<Snapshot, String> {
    let mut snapshot: Snapshot = serde_json::from_slice(bytes)
        .map_err(|e| format!("Keep returned an invalid snapshot: {e}"))?;
    for note in &mut snapshot.notes {
        if let Some(preview) = note.preview.take() {
            let png = base64::engine::general_purpose::STANDARD
                .decode(preview)
                .map_err(|e| format!("Keep returned an invalid note preview: {e}"))?;
            note.preview_handle = Some(iced::widget::image::Handle::from_bytes(png));
        }
    }
    Ok(snapshot)
}
pub fn signin() -> Result<(), String> {
    run("keep-signin").map(|_| ())
}
fn run(subcommand: &str) -> Result<Vec<u8>, String> {
    for path in helper_candidates() {
        match Command::new(&path)
            .arg(subcommand)
            .env_remove("KEEP_BRIDGE_HEADED")
            .env_remove("TASKS_BRIDGE_HEADED")
            .stdin(Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => return Ok(output.stdout),
            Ok(output) => {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(if detail.contains("Unknown command") {
                    "The Google Keep helper needs rebuilding: run deno task compile in tools/keep-bridge.".into()
                } else if detail.is_empty() {
                    format!("The Google Keep helper exited with {}", output.status)
                } else {
                    detail
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Could not launch the Google Keep helper: {e}")),
        }
    }
    Err("Google Keep helper is missing. Build it with `deno task compile` in tools/keep-bridge, then reconnect.".into())
}
fn helper_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = std::env::var_os("TIMESLAY_KEEP_BRIDGE")
        .or_else(|| std::env::var_os("TIMESLAY_TASKS_BRIDGE"))
    {
        return vec![PathBuf::from(path)];
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from);

        // Installed: beside the application binary.
        if let Some(dir) = &dir {
            candidates.push(dir.join("keep-bridge"));
        }

        // Run from a checkout: the app is `<crate>/target/{debug,release}/app`
        // and a test binary is `<crate>/target/debug/deps/test`, so walk up a
        // few levels. This keeps discovery independent of the working
        // directory, which a launcher or IDE does not set to the crate root.
        for _ in 0..3 {
            dir = dir.and_then(|parent| parent.parent().map(PathBuf::from));
            if let Some(dir) = &dir {
                candidates.push(dir.join("tools/keep-bridge/keep-bridge"));
            }
        }
    }

    // Running with the crate root as the working directory.
    candidates.push(PathBuf::from("tools/keep-bridge/keep-bridge"));

    // Last resort: anything named `keep-bridge` on PATH.
    candidates.push(PathBuf::from("keep-bridge"));

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    fn note() -> Note {
        Note {
            title: "Shopping list".into(),
            description: "Buy coffee and crème".into(),
            ..Note::default()
        }
    }
    #[test]
    fn substring_and_typos() {
        for q in ["HOPP", "coffee", "cofee", "shoppnig", "CRÈME"] {
            assert!(score(&note(), q).is_some(), "{q}");
        }
        for q in ["unrelated", "zz", "coffee unrelated"] {
            assert!(score(&note(), q).is_none(), "{q}");
        }
        assert!(score(&note(), "Shopping").unwrap() < score(&note(), "shoppnig").unwrap());
    }
    #[test]
    fn scopes_and_labels_intersect_with_search() {
        let mut n = note();
        n.archived = true;
        n.labels = vec!["Work".into()];
        let mut store = Store::default();
        store.snapshot.notes.push(n);
        assert!(store.visible().is_empty());
        store.scope = Scope::All;
        store.label = Some("work".into());
        store.query = "cofee".into();
        assert_eq!(store.visible().len(), 1);
        store.label = Some("Home".into());
        assert!(store.visible().is_empty());
    }
}
