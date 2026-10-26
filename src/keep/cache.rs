//! Persist complete, validated snapshots. A failed refresh leaves the last one intact.
use super::Snapshot;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

pub(super) fn load(path: &Path) -> Option<Snapshot> {
    let result = fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| super::decode(&bytes));
    match result {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            if path.exists() {
                log::warn!("could not load Keep cache: {error}");
            }
            None
        }
    }
}

pub(super) fn accept(bytes: &[u8], path: Option<&Path>) -> Result<Snapshot, String> {
    // Validate both JSON and previews before replacing the previous snapshot.
    let snapshot = super::decode(bytes)?;
    if let Some(path) = path {
        if let Err(error) = save(path, bytes) {
            log::warn!("could not save Keep cache: {error}");
        }
    }
    Ok(snapshot)
}

fn save(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let temporary = path.with_extension(format!("{:032x}.tmp", u128::from_ne_bytes(random)));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keep::Store;

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            Self(
                std::env::temp_dir()
                    .join(format!("keep-cache-{:032x}", u128::from_ne_bytes(random))),
            )
        }
        fn path(&self) -> std::path::PathBuf {
            self.0.join("keep/notes.json")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn snapshot(title: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "notes": [{"id": "note", "url": "https://keep.google.com/#NOTE/note",
                "title": title, "description": "Body", "labels": ["Work"],
                "color": "rgb(255, 248, 184)", "pinned": true, "archived": true,
                "checklist": [{"text": "Item", "checked": true, "level": 1}],
                "metadata": ["Reminder"], "preview": "aGVsbG8=",
                "details": {"created": "Sep 17, 2026", "tags": ["routine"]},
                "body": [{"text": "Bold body", "bold": true}]}],
            "warnings": ["A warning"]
        }))
        .unwrap()
    }

    #[test]
    fn restores_complete_snapshot_across_stores_and_replaces_it_on_refresh() {
        let scratch = Scratch::new();
        let path = scratch.path();
        accept(&snapshot("First"), Some(&path)).unwrap();
        let mut store = Store::default();
        store.cache_in(Some(path.clone()));
        let note = &store.snapshot.notes[0];
        assert_eq!(note.title, "First");
        assert_eq!(note.description, "Body");
        assert_eq!(note.labels, ["Work"]);
        assert!(note.pinned && note.archived);
        assert!(note.checklist[0].checked);
        assert_eq!(note.checklist[0].level, 1);
        assert_eq!(note.metadata, ["Reminder"]);
        assert_eq!(
            note.parsed_metadata().created.as_deref(),
            Some("Sep 17, 2026")
        );
        assert_eq!(note.parsed_metadata().tags, ["routine"]);
        assert_eq!(note.body[0].text, "Bold body");
        assert!(note.body[0].bold);
        assert!(note.preview_handle.is_some());
        assert_eq!(store.snapshot.warnings, ["A warning"]);
        accept(&snapshot("Updated"), Some(&path)).unwrap();
        store.cache_in(Some(path));
        assert_eq!(store.snapshot.notes[0].title, "Updated");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(scratch.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_refresh_does_not_replace_last_good_cache() {
        let scratch = Scratch::new();
        let path = scratch.path();
        accept(&snapshot("Saved"), Some(&path)).unwrap();
        for invalid in [
            b"broken json".as_slice(),
            br#"{"notes":[{"preview":"%%%"}]}"#,
        ] {
            assert!(accept(invalid, Some(&path)).is_err());
            assert_eq!(load(&path).unwrap().notes[0].title, "Saved");
        }
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn absent_or_corrupt_cache_is_ignored_and_unwritable_cache_allows_fresh_notes() {
        let scratch = Scratch::new();
        let path = scratch.path();
        let mut store = Store::default();
        store.cache_in(Some(path.clone()));
        assert!(store.snapshot.notes.is_empty());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"broken").unwrap();
        store.cache_in(Some(path.clone()));
        assert!(store.snapshot.notes.is_empty());
        let impossible = path.join("notes.json");
        assert_eq!(
            accept(&snapshot("Fresh"), Some(&impossible)).unwrap().notes[0].title,
            "Fresh"
        );
    }
}
