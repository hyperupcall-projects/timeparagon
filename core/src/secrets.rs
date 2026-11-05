//! Credentials: the Google sign-in and the Gmail accounts' tokens.
//!
//! They are kept out of the database, in `$XDG_STATE_HOME/timeslay/secrets.json`,
//! readable by this user only. Each kind is one key of a JSON object. Android
//! keeps its Google tokens encrypted elsewhere: see `oauth::android_tokens`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Once;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

pub mod keys {
    pub const GOOGLE_TOKENS: &str = "google_tokens";
    pub const GMAIL_ACCOUNTS: &str = "gmail_accounts";
}

/// The secrets file on this device, with credentials older builds kept
/// elsewhere already moved into it.
pub fn path() -> Option<PathBuf> {
    static MOVE: Once = Once::new();
    let directories = crate::config::project_dirs()?;
    let path = directories
        .state_dir()
        .unwrap_or_else(|| directories.data_local_dir())
        .join("secrets.json");
    MOVE.call_once(|| {
        let database = crate::task_db::path();
        if let Err(error) = adopt(&path, database.as_deref(), directories.config_dir()) {
            log::warn!(
                "could not move older credentials into {}: {error}",
                path.display()
            );
        }
    });
    Some(path)
}

/// Everything in the file. A missing file holds nothing; one that does not
/// parse is an error, so a write cannot replace the secrets in it.
fn read_all(path: &Path) -> io::Result<Map<String, Value>> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(error),
    }
}

pub fn read<T: DeserializeOwned>(path: &Path, key: &str) -> io::Result<Option<T>> {
    read_all(path)?
        .remove(key)
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .transpose()
}

/// Sets or, with `None`, removes one key, keeping the others. Writers in every
/// process take turns.
fn change(path: &Path, key: &str, value: Option<Value>) -> io::Result<()> {
    crate::fsutil::locked(path, || {
        let mut all = read_all(path)?;
        match value {
            Some(value) => all.insert(key.to_owned(), value),
            None if all.contains_key(key) => all.remove(key),
            None => return Ok(()),
        };
        let text = serde_json::to_vec_pretty(&all).map_err(io::Error::other)?;
        crate::fsutil::write_atomic(path, &text, crate::fsutil::Kind::Private)
    })
}

pub fn write<T: Serialize + ?Sized>(path: &Path, key: &str, value: &T) -> io::Result<()> {
    let value = serde_json::to_value(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    change(path, key, Some(value))
}

pub fn remove(path: &Path, key: &str) -> io::Result<()> {
    change(path, key, None)
}

/// Moves credentials here from where older builds kept them: the database,
/// where a build of the same release briefly put them, and before that a TOML
/// file each in the config directory. What this file already has is kept. A
/// copy is removed once this file holds it, and the database is rebuilt, so
/// no earlier version of a token lingers in its free pages.
fn adopt(path: &Path, database: Option<&Path>, config: &Path) -> io::Result<()> {
    let present = read_all(path)?;
    let mut removed = false;
    for key in [keys::GOOGLE_TOKENS, keys::GMAIL_ACCOUNTS] {
        let Some(database) = database else { break };
        let Some(value) = crate::task_db::read_document::<Value>(database, key)? else {
            continue;
        };
        if !present.contains_key(key) {
            change(path, key, Some(value))?;
        }
        remove_from_database(database, key)?;
        removed = true;
    }
    if removed && let Some(database) = database {
        crate::task_db::Database::open(database)
            .map_err(io::Error::other)?
            .connection()
            .execute_batch("VACUUM;")
            .map_err(io::Error::other)?;
    }

    #[cfg(not(target_os = "android"))]
    for (key, file) in [
        (keys::GOOGLE_TOKENS, "google-token.toml"),
        (keys::GMAIL_ACCOUNTS, "gmail-accounts.toml"),
    ] {
        let file = config.join(file);
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if read_all(path)?.contains_key(key) {
            continue;
        }
        let value: Value = toml::from_str(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        change(path, key, Some(value))?;
        fs::remove_file(&file)?;
    }
    #[cfg(target_os = "android")]
    let _ = config;
    Ok(())
}

fn remove_from_database(database: &Path, key: &str) -> io::Result<()> {
    let mut database = crate::task_db::Database::open(database).map_err(io::Error::other)?;
    database
        .connection()
        .execute("DELETE FROM documents WHERE key=?1", [key])
        .map(|_| ())
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_privately_and_keep_each_other() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state").join("secrets.json");
        write(&path, keys::GOOGLE_TOKENS, &"google").unwrap();
        write(&path, keys::GMAIL_ACCOUNTS, &"gmail").unwrap();
        remove(&path, keys::GOOGLE_TOKENS).unwrap();
        assert_eq!(read::<String>(&path, keys::GOOGLE_TOKENS).unwrap(), None);
        assert_eq!(
            read::<String>(&path, keys::GMAIL_ACCOUNTS)
                .unwrap()
                .as_deref(),
            Some("gmail")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_file_that_does_not_parse_is_never_written_over() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets.json");
        fs::write(&path, "not json").unwrap();
        assert!(write(&path, keys::GOOGLE_TOKENS, &"x").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
    }

    #[test]
    fn credentials_leave_the_database_and_the_old_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("secrets.json");
        let database = directory.path().join("test.sqlite3");
        let config = directory.path().join("config");
        fs::create_dir_all(&config).unwrap();
        // An earlier refresh leaves the old token behind in a free page.
        let old = "an earlier token ".repeat(300);
        crate::task_db::write_document(&database, keys::GOOGLE_TOKENS, &old).unwrap();
        crate::task_db::write_document(&database, keys::GOOGLE_TOKENS, &"from the database")
            .unwrap();
        fs::write(
            config.join("google-token.toml"),
            "access_token = \"older\"\n",
        )
        .unwrap();
        fs::write(
            config.join("gmail-accounts.toml"),
            "[[accounts]]\nemail = \"a\"\n",
        )
        .unwrap();

        adopt(&path, Some(&database), &config).unwrap();

        assert_eq!(
            read::<String>(&path, keys::GOOGLE_TOKENS)
                .unwrap()
                .as_deref(),
            Some("from the database")
        );
        let gmail: Value = read(&path, keys::GMAIL_ACCOUNTS).unwrap().unwrap();
        assert_eq!(gmail["accounts"][0]["email"], "a");
        assert_eq!(
            crate::task_db::read_document::<Value>(&database, keys::GOOGLE_TOKENS).unwrap(),
            None
        );
        assert!(!config.join("gmail-accounts.toml").exists());
        // The database's copy was newer, so the old file is left for the user.
        assert!(config.join("google-token.toml").exists());
        let bytes = fs::read(&database).unwrap();
        assert!(
            !bytes
                .windows(17)
                .any(|window| window == b"from the database"),
            "the deleted secret is overwritten"
        );
    }
}
