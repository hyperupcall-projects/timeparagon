//! Storage Access Framework bridge. The shared Markdown codec still owns file
//! naming and front matter; only document I/O is delegated to Android.
use super::{Library, Saved, files};
use jni::objects::{JClass, JObject, JString, JValue};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, path::Path};

pub fn is_tree(path: &Path) -> bool {
    path.to_string_lossy().starts_with("content://")
}

pub fn request(request: Value) -> io::Result<Value> {
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.map_err(io::Error::other)?;
    let mut env = vm.attach_current_thread().map_err(io::Error::other)?;
    let result = env.with_local_frame(32, |env| -> jni::errors::Result<String> {
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        let loader = env
            .call_method(
                &activity,
                "getClassLoader",
                "()Ljava/lang/ClassLoader;",
                &[],
            )?
            .l()?;
        let name = env.new_string("dev.timeslay.day.notes.NotesStorage")?;
        let class = env
            .call_method(
                loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&name)],
            )?
            .l()?;
        let input = env.new_string(request.to_string())?;
        let output = env
            .call_static_method(
                JClass::from(class),
                "run",
                "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&activity), JValue::Object(&input)],
            )?
            .l()?;
        Ok(env.get_string(&JString::from(output))?.into())
    });
    if result.is_err() {
        let _ = env.exception_clear();
    }
    let value: Value =
        serde_json::from_str(&result.map_err(io::Error::other)?).map_err(io::Error::other)?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(io::Error::other(error.to_owned()));
    }
    Ok(value)
}

#[derive(Deserialize)]
struct Document {
    name: String,
    text: String,
    modified: i64,
}
fn documents(directory: &Path) -> io::Result<Vec<Document>> {
    serde_json::from_value(request(json!({"op":"read", "tree":directory}))?["files"].clone())
        .map_err(io::Error::other)
}
pub(super) fn read(directory: &Path) -> io::Result<Library> {
    Ok(Library {
        notes: documents(directory)?
            .into_iter()
            .map(|doc| files::parse(&doc.name, &doc.text, doc.modified))
            .collect(),
        removed: Vec::new(),
    })
}
pub(super) fn fingerprint(directory: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    request(json!({"op":"stat", "tree":directory}))
        .ok()
        .map(|v| v.to_string())
        .hash(&mut hash);
    hash.finish()
}
fn stage(directory: &Path) -> io::Result<(tempfile::TempDir, BTreeMap<String, String>)> {
    let scratch = tempfile::tempdir()?;
    let mut before = BTreeMap::new();
    for doc in documents(directory)? {
        // Providers are untrusted: never use a display name as a relative path.
        if Path::new(&doc.name).file_name().and_then(|n| n.to_str()) != Some(doc.name.as_str()) {
            return Err(io::Error::other("Invalid document name"));
        }
        std::fs::write(scratch.path().join(&doc.name), &doc.text)?;
        before.insert(doc.name, doc.text);
    }
    Ok((scratch, before))
}

pub(super) fn claim_identical(library: &mut Library, directory: &Path) -> io::Result<()> {
    let (scratch, _) = stage(directory)?;
    files::claim_identical(library, scratch.path())
}

pub(super) fn save(library: &Library, directory: &Path) -> io::Result<Saved> {
    let (scratch, before) = stage(directory)?;
    let saved = files::write(library, scratch.path())?;
    let mut writes = BTreeMap::new();
    for (_, name) in &saved.files {
        let text = std::fs::read_to_string(scratch.path().join(name))?;
        if before.get(name) != Some(&text) {
            writes.insert(name, text);
        }
    }
    let removed: Vec<_> = before
        .keys()
        .filter(|name| !scratch.path().join(name).exists())
        .collect();
    request(json!({"op":"write", "tree":directory, "writes":writes, "removed":removed}))?;
    Ok(saved)
}
pub fn open(directory: &Path, file: &str) -> io::Result<()> {
    request(json!({"op":"open", "tree":directory, "file":file})).map(|_| ())
}
