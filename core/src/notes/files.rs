//! How a note is kept on disk: `<title>.md`, holding the note's Markdown.
//!
//! A note with a color other than the default, or with connections, starts
//! with front matter, which other Markdown editors show as properties:
//!
//! ```text
//! ---
//! color: rose
//! links:
//!   - {"kind":"habit","id":"h1","title":"Read"}
//! ---
//! The note itself.
//! ```
//!
//! Each connection is a JSON object, which is also valid YAML. Connections
//! another editor rewrote as YAML mappings (`- kind: habit` and so on) are read
//! too. Front matter lines this app does not know are kept and written back.

use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{Library, Link, Note, Pastel, Saved, UNTITLED};

const EXTENSION: &str = "md";
/// Leaves room under the usual 255-byte file name limit for the extension and
/// a " 12" to tell apart notes with the same title.
const MAX_STEM_BYTES: usize = 200;

/// The file name, without its extension, for a note with this title.
pub(super) fn file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            // Not allowed in a file name here or on another system.
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // A leading dot would hide the file.
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let mut stem = if trimmed.is_empty() {
        UNTITLED.to_owned()
    } else {
        trimmed.to_owned()
    };
    if stem.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        stem.truncate(end);
        stem.truncate(stem.trim_end().len());
    }
    stem
}

/// `name` without its `.md`, when it is a Markdown file's name.
fn markdown_stem(name: &str) -> Option<&str> {
    let (stem, extension) = name.rsplit_once('.')?;
    (extension.eq_ignore_ascii_case(EXTENSION) && !stem.is_empty() && !name.starts_with('.'))
        .then_some(stem)
}

/// The `n`th name for notes titled `stem`: `stem.md`, `stem 2.md`, ...
fn numbered(stem: &str, n: usize) -> String {
    if n <= 1 {
        format!("{stem}.{EXTENSION}")
    } else {
        format!("{stem} {n}.{EXTENSION}")
    }
}

/// Whether `file` is one of the names a note titled `stem` may have.
fn fits(file: &str, stem: &str) -> bool {
    let Some(own) = markdown_stem(file) else {
        return false;
    };
    own == stem
        || own
            .strip_prefix(stem)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|n| n.parse::<usize>().ok())
            .is_some_and(|n| n >= 2)
}

/// The Markdown files in the directory, by name.
fn markdown_files(directory: &Path) -> io::Result<Vec<(String, fs::Metadata)>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if markdown_stem(&name).is_none() {
            continue;
        }
        // Follows symbolic links.
        match fs::metadata(entry.path()) {
            Ok(metadata) if metadata.is_file() => files.push((name, metadata)),
            _ => {}
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}

pub(super) fn read(directory: &Path) -> io::Result<Library> {
    let mut notes = Vec::new();
    for (name, metadata) in markdown_files(directory)? {
        let text = match fs::read_to_string(directory.join(&name)) {
            Ok(text) => text,
            Err(error) => {
                log::warn!("the note {name} could not be read: {error}");
                continue;
            }
        };
        let modified_ms = metadata.modified().map_or(0, millis);
        notes.push(parse(&name, &text, modified_ms));
    }
    Ok(Library {
        notes,
        removed: Vec::new(),
    })
}

pub(super) fn fingerprint(directory: &Path) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Ok(files) = markdown_files(directory) {
        for (name, metadata) in files {
            name.hash(&mut hasher);
            metadata.len().hash(&mut hasher);
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .hash(&mut hasher);
        }
    } else {
        u64::MAX.hash(&mut hasher);
    }
    hasher.finish()
}

/// Splits off front matter: a first line of `---` and the next line of `---`.
fn split_front_matter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return (None, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return (Some(&rest[..offset]), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, text)
}

/// A YAML scalar as plain text: quotes removed.
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        serde_json::from_str(value).unwrap_or_else(|_| value[1..value.len() - 1].to_owned())
    } else if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.to_owned()
    }
}

/// The connections under `links:`: a JSON array on the same line, or a list
/// of JSON objects or YAML mappings on the lines after.
fn parse_links(value: &str, lines: &[&str]) -> Option<Vec<Link>> {
    let value = value.trim();
    if !value.is_empty() {
        return serde_json::from_str(value).ok();
    }
    let mut items: Vec<Vec<&str>> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(first) = trimmed.strip_prefix('-') {
            items.push(vec![first.trim()]);
        } else {
            items.last_mut()?.push(trimmed);
        }
    }
    items
        .into_iter()
        .map(|item| {
            if item.len() == 1 && item[0].starts_with('{') {
                return serde_json::from_str(item[0]).ok();
            }
            let mut object = serde_json::Map::new();
            for pair in item.into_iter().filter(|pair| !pair.is_empty()) {
                let (key, value) = pair.split_once(':')?;
                object.insert(key.trim().to_owned(), unquote(value).into());
            }
            serde_json::from_value(object.into()).ok()
        })
        .collect()
}

fn parse_front_matter(front: &str, note: &mut Note) {
    // Each top-level key with the indented or list lines under it.
    let mut entries: Vec<(Option<&str>, Vec<&str>)> = Vec::new();
    for line in front.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let nested =
            content.is_empty() || content.starts_with([' ', '\t']) || content.starts_with('-');
        match entries.last_mut() {
            Some((Some(_), lines)) if nested => lines.push(line),
            _ => {
                let key = (!nested)
                    .then(|| content.split_once(':').map(|(key, _)| key.trim()))
                    .flatten();
                entries.push((key, vec![line]));
            }
        }
    }
    for (key, lines) in entries {
        let first = lines[0].trim_end_matches(['\r', '\n']);
        let value = first.split_once(':').map_or("", |(_, value)| value);
        let understood = match key {
            Some("color") => Pastel::ALL
                .into_iter()
                .find(|color| color.name().eq_ignore_ascii_case(&unquote(value)))
                .map(|color| note.color = color)
                .is_some(),
            Some("links") => {
                let parsed = parse_links(value, &lines[1..]);
                if parsed.is_none() {
                    log::warn!(
                        "the connections of the note {} could not be read",
                        note.file.as_deref().unwrap_or_default()
                    );
                }
                parsed.map(|links| note.links = links).is_some()
            }
            _ => false,
        };
        if !understood {
            for line in lines {
                note.extra.push_str(line);
            }
        }
    }
    if !note.extra.is_empty() && !note.extra.ends_with('\n') {
        note.extra.push('\n');
    }
}

pub(super) fn parse(name: &str, text: &str, modified_ms: i64) -> Note {
    let (front, body) = split_front_matter(text);
    let mut note = Note {
        id: format!("file:{name}"),
        title: markdown_stem(name).unwrap_or(name).to_owned(),
        body: body.to_owned(),
        color: Pastel::default(),
        links: Vec::new(),
        modified_ms,
        revision: 0,
        file: Some(name.to_owned()),
        extra: String::new(),
    };
    if let Some(front) = front {
        parse_front_matter(front, &mut note);
    }
    note
}

/// The file's contents for a note.
pub(super) fn render(note: &Note) -> String {
    let mut front = String::new();
    if note.color != Pastel::default() {
        front.push_str(&format!("color: {}\n", note.color.name().to_lowercase()));
    }
    if !note.links.is_empty() {
        front.push_str("links:\n");
        for link in &note.links {
            let json = serde_json::to_string(link).unwrap_or_default();
            front.push_str(&format!("  - {json}\n"));
        }
    }
    front.push_str(&note.extra);
    if !front.is_empty() && !front.ends_with('\n') {
        front.push('\n');
    }
    // A note that itself starts with a `---` line gets empty front matter, so
    // it is not read back as front matter.
    if front.is_empty() && split_front_matter(&note.body).0.is_none() {
        note.body.clone()
    } else {
        format!("---\n{front}---\n{}", note.body)
    }
}

pub(super) fn write(library: &Library, directory: &Path) -> io::Result<Saved> {
    fs::create_dir_all(directory)?;
    let mut saved = Saved::default();
    for file in &library.removed {
        // Another note has taken the name since.
        let reused = library
            .notes
            .iter()
            .any(|note| note.file.as_deref() == Some(file.as_str()));
        if !reused {
            match fs::remove_file(directory.join(file)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        saved.removed.push(file.clone());
    }

    // Compared without case, since the phone's storage ignores it.
    let owned: HashSet<String> = library
        .notes
        .iter()
        .filter_map(|note| note.file.as_deref())
        .map(str::to_lowercase)
        .collect();
    let mut taken = HashSet::new();
    let mut chosen: Vec<Option<String>> = vec![None; library.notes.len()];
    // A note whose file still fits its title keeps it, before any other note
    // picks a name.
    for (index, note) in library.notes.iter().enumerate() {
        if let Some(file) = &note.file
            && fits(file, &file_stem(&note.title))
            && taken.insert(file.to_lowercase())
        {
            chosen[index] = Some(file.clone());
        }
    }
    for (index, note) in library.notes.iter().enumerate() {
        if chosen[index].is_some() {
            continue;
        }
        let stem = file_stem(&note.title);
        let own = note.file.as_deref().map(str::to_lowercase);
        let name = (1..)
            .map(|n| numbered(&stem, n))
            .find(|name| {
                let key = name.to_lowercase();
                if taken.contains(&key) {
                    return false;
                }
                // Its own old name differing only in case is free.
                own.as_ref() == Some(&key)
                    || (!owned.contains(&key) && !directory.join(name).exists())
            })
            .expect("some numbered name is free");
        taken.insert(name.to_lowercase());
        chosen[index] = Some(name);
    }

    for (note, name) in library.notes.iter().zip(chosen) {
        let name = name.expect("every note has a name");
        let path = directory.join(&name);
        if let Some(old) = &note.file
            && *old != name
        {
            match fs::rename(directory.join(old), &path) {
                Ok(()) => {}
                // Removed by something else meanwhile: written afresh below.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        let contents = render(note);
        if fs::read(&path).ok().as_deref() != Some(contents.as_bytes()) {
            crate::fsutil::write_atomic(&path, contents.as_bytes(), crate::fsutil::Kind::Document)?;
        }
        saved.files.push((note.id.clone(), name));
    }
    Ok(saved)
}

/// Gives each note not yet in the directory the file already there with
/// exactly its contents, so an interrupted migration does not write it twice.
pub(super) fn claim_identical(library: &mut Library, directory: &Path) -> io::Result<()> {
    let mut claimed = HashSet::new();
    for note in library.notes.iter_mut().filter(|note| note.file.is_none()) {
        let stem = file_stem(&note.title);
        let contents = render(note);
        for n in 1.. {
            let name = numbered(&stem, n);
            let path = directory.join(&name);
            if !path.exists() {
                break;
            }
            if !claimed.contains(&name) && fs::read(&path)? == contents.as_bytes() {
                claimed.insert(name.clone());
                note.file = Some(name);
                break;
            }
        }
    }
    Ok(())
}

/// Sets a file's modification time. A failure only costs the sort order.
pub(super) fn set_modified(path: &Path, modified_ms: i64) {
    let Ok(ms) = u64::try_from(modified_ms) else {
        return;
    };
    if ms == 0 {
        return;
    }
    let result = fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(UNIX_EPOCH + Duration::from_millis(ms)));
    if let Err(error) = result {
        log::warn!("could not set the time of {}: {error}", path.display());
    }
}
