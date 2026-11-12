//! Credentials: Google sign-in, Gmail accounts, and the OpenRouter API token.
//!
//! They are kept out of the database, in `$XDG_STATE_HOME/timeslay/secrets.json`,
//! readable by this user only. Each kind is one key of a JSON object. Android
//! keeps its Google tokens encrypted elsewhere: see `oauth::android_tokens`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

pub mod keys {
	pub const OPENROUTER_TOKEN: &str = "openrouter_token";
	pub const GOOGLE_TOKENS: &str = "google_tokens";
	pub const GMAIL_ACCOUNTS: &str = "gmail_accounts";
}

/// The secrets file on this device.
pub fn path() -> Option<PathBuf> {
	let directories = crate::config::project_dirs()?;
	Some(
		directories
			.state_dir()
			.unwrap_or_else(|| directories.data_local_dir())
			.join("secrets.json"),
	)
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn secrets_round_trip_privately_and_keep_each_other() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("state").join("secrets.json");
		write(&path, keys::GOOGLE_TOKENS, &"google").unwrap();
		write(&path, keys::GMAIL_ACCOUNTS, &"gmail").unwrap();
		write(&path, keys::OPENROUTER_TOKEN, &"openrouter").unwrap();
		assert_eq!(
			read::<String>(&path, keys::OPENROUTER_TOKEN)
				.unwrap()
				.as_deref(),
			Some("openrouter")
		);
		remove(&path, keys::OPENROUTER_TOKEN).unwrap();
		assert_eq!(read::<String>(&path, keys::OPENROUTER_TOKEN).unwrap(), None);

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
}
