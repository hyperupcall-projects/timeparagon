//! Replacing a file without ever exposing half of it.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use tempfile::Builder;

/// Who may read the file and how hard to try to keep it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
	/// Settings, credentials and anything else that cannot be rebuilt: readable
	/// by this user only, and on disk before the call returns.
	Private,
	/// A cache that a later sync can rebuild. Follows the umask, and is not
	/// flushed, since losing it to a crash only costs a re-fetch.
	Cache,
	/// The user's own document, such as a note: follows the umask like any
	/// file they make, and is on disk before the call returns.
	Document,
}

/// Writes `bytes` to `path` through a temporary file beside it, so a crash
/// mid-write leaves the old contents. Each call gets its own temporary name,
/// so processes writing the same file at once cannot truncate each other.
pub fn write_atomic(path: &Path, bytes: &[u8], kind: Kind) -> io::Result<()> {
	write_atomic_with(path, bytes, kind, |_| Ok(()))
}

/// [`write_atomic`], with a last look at the finished file before it replaces
/// the old one. An error from `finish` leaves the old file in place.
pub fn write_atomic_with(
	path: &Path,
	bytes: &[u8],
	kind: Kind,
	finish: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
	let parent = path
		.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or_else(|| Path::new("."));
	fs::create_dir_all(parent)?;

	let name = path.file_name().unwrap_or_default().to_string_lossy();
	let prefix = format!(".{name}.");
	let mut builder = Builder::new();
	builder.prefix(&prefix).suffix(".tmp");
	#[cfg(unix)]
	if kind != Kind::Private {
		use std::os::unix::fs::PermissionsExt;
		// Requested at creation, so the kernel applies the umask.
		builder.permissions(fs::Permissions::from_mode(0o666));
	}

	// Removes itself if anything below fails.
	let mut temporary = builder.tempfile_in(parent)?;
	temporary.write_all(bytes)?;
	finish(temporary.as_file())?;
	if kind != Kind::Cache {
		temporary.as_file().sync_all()?;
	}
	temporary.persist(path).map_err(|error| error.error)?;
	if kind != Kind::Cache {
		// The rename itself has to reach the disk as well.
		File::open(parent)?.sync_all()?;
	}
	Ok(())
}

/// Runs `work` holding an exclusive lock shared by every process that goes
/// through this function for `path`. The lock lives in a file beside it, since
/// `path` itself gets replaced. It is not reentrant: `work` must not lock the
/// same path again. If locking is unavailable the work still runs, because a
/// save without a lock is better than no save.
pub fn locked<T>(path: &Path, work: impl FnOnce() -> T) -> T {
	let mut name = path.file_name().unwrap_or_default().to_os_string();
	name.push(".lock");
	let lock = path.with_file_name(name);
	let _held = (|| {
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)?;
		}
		let mut options = fs::OpenOptions::new();
		options.create(true).truncate(false).write(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let file = options.open(&lock)?;
		file.lock()?;
		io::Result::Ok(file)
	})();
	if let Err(error) = &_held {
		log::warn!("could not lock {}: {error}", lock.display());
	}
	work()
}

/// Takes access away from everyone but this user if `path` has it, for
/// files written before they were created private.
#[cfg(unix)]
pub fn make_private(path: &Path) -> io::Result<()> {
	use std::os::unix::fs::PermissionsExt;

	let mode = fs::metadata(path)?.permissions().mode();
	if mode & 0o077 != 0 {
		fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o700))?;
	}
	Ok(())
}

#[cfg(not(unix))]
pub fn make_private(_path: &Path) -> io::Result<()> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::os::unix::fs::PermissionsExt;

	fn mode(path: &Path) -> u32 {
		fs::metadata(path).unwrap().permissions().mode() & 0o777
	}

	#[test]
	fn a_private_file_is_readable_only_by_this_user_even_when_replacing_an_open_one() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("nested").join("config.toml");
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(&path, "old").unwrap();
		fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();

		write_atomic(&path, b"new", Kind::Private).unwrap();

		assert_eq!(fs::read_to_string(&path).unwrap(), "new");
		assert_eq!(mode(&path), 0o600);
	}

	#[test]
	fn a_cache_follows_the_umask_rather_than_being_private() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("2026-10.json");
		write_atomic(&path, b"{}", Kind::Cache).unwrap();

		let created = dir.path().join("plain");
		fs::write(&created, b"").unwrap();
		assert_eq!(mode(&path), mode(&created));
	}

	#[test]
	fn a_failed_finish_keeps_the_old_file_and_leaves_no_litter() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.toml");
		fs::write(&path, "old").unwrap();

		let result = write_atomic_with(&path, b"new", Kind::Private, |_| {
			Err(io::Error::other("no"))
		});

		assert!(result.is_err());
		assert_eq!(fs::read_to_string(&path).unwrap(), "old");
		assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
	}

	#[test]
	fn concurrent_writers_never_expose_a_torn_file() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.toml");
		let contents: Vec<Vec<u8>> = (0..4u8).map(|n| vec![b'a' + n; 256 * 1024]).collect();
		fs::write(&path, &contents[0]).unwrap();

		std::thread::scope(|scope| {
			for bytes in &contents {
				let path = path.clone();
				scope.spawn(move || {
					for _ in 0..10 {
						write_atomic(&path, bytes, Kind::Cache).unwrap();
					}
				});
			}
			for _ in 0..200 {
				let seen = fs::read(&path).unwrap();
				assert!(contents.contains(&seen), "read a torn file");
			}
		});
	}

	#[test]
	fn locked_sections_for_a_path_do_not_overlap() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.toml");
		let inside = std::sync::atomic::AtomicUsize::new(0);
		std::thread::scope(|scope| {
			for _ in 0..4 {
				scope.spawn(|| {
					for _ in 0..25 {
						locked(&path, || {
							use std::sync::atomic::Ordering::SeqCst;
							assert_eq!(inside.fetch_add(1, SeqCst), 0);
							std::thread::yield_now();
							inside.fetch_sub(1, SeqCst);
						});
					}
				});
			}
		});
	}

	#[test]
	fn make_private_only_removes_access_for_others() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("config.toml");
		fs::write(&path, "x").unwrap();
		fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
		make_private(&path).unwrap();
		assert_eq!(mode(&path), 0o600);
	}
}
