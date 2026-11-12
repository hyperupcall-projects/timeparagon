//! This links core as an ordinary dependency, without cfg(test) or opt-in features.
use timeslay_core::{calendar, config, secrets, task_db};

#[test]
fn default_storage_is_temporary() {
	assert!(
		config::isolated(),
		"integration tests must always be isolated"
	);
	let dirs = config::project_dirs().unwrap();
	let root = dirs.config_dir().parent().unwrap();
	assert!(root.starts_with(std::env::temp_dir()));
	assert!(
		root.file_name()
			.unwrap()
			.to_str()
			.unwrap()
			.starts_with("timeslay-test-")
	);
	for path in [
		dirs.config_dir(),
		dirs.data_dir(),
		dirs.data_local_dir(),
		dirs.cache_dir(),
		dirs.state_dir().unwrap(),
	] {
		assert!(path.starts_with(root));
	}
	let database = task_db::path().unwrap();
	assert!(database.starts_with(root));
	assert!(config::config_path().unwrap().starts_with(root));
	assert!(secrets::path().unwrap().starts_with(root));
	task_db::write_document(&database, "isolation_probe", &"temporary").unwrap();
	assert_eq!(
		task_db::read_document::<String>(&database, "isolation_probe")
			.unwrap()
			.as_deref(),
		Some("temporary")
	);
	calendar::operations::submit(
		calendar::operations::Job::Delete {
			calendar: "tasks:inbox".into(),
			id: "task:a".into(),
			scope: calendar::SaveScope::Current,
			notify: false,
			with_tasks: None,
		},
		None,
		"tasks:inbox",
		"task:a",
	)
	.unwrap();
	// The write is answered locally, without reaching Google.
	assert!(
		calendar::operations::log_since(0)
			.iter()
			.any(|entry| entry.text.starts_with("Saved to Google: Delete event"))
	);
	assert_eq!(calendar::operations::sending(), 0);
}

#[test]
#[cfg(unix)]
fn default_writes_leave_existing_user_storage_untouched() {
	let home = tempfile::tempdir().unwrap();
	let mut sentinels = Vec::new();
	for directory in ["config", "data", "cache", "state"] {
		let path = home.path().join(directory).join("timeslay");
		std::fs::create_dir_all(&path).unwrap();
		for file in [
			"timeslay.sqlite3",
			"tasks.sqlite3",
			"secrets.json",
			"config.toml",
		] {
			let file = path.join(file);
			std::fs::write(&file, b"existing user data: do not modify").unwrap();
			sentinels.push(file);
		}
	}
	let output = std::process::Command::new(std::env::current_exe().unwrap())
		.args(["--exact", "default_storage_is_temporary", "--nocapture"])
		.env("HOME", home.path())
		.env("XDG_CONFIG_HOME", home.path().join("config"))
		.env("XDG_DATA_HOME", home.path().join("data"))
		.env("XDG_CACHE_HOME", home.path().join("cache"))
		.env("XDG_STATE_HOME", home.path().join("state"))
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"{}\n{}",
		String::from_utf8_lossy(&output.stdout),
		String::from_utf8_lossy(&output.stderr)
	);
	for file in sentinels {
		assert_eq!(
			std::fs::read(&file).unwrap(),
			b"existing user data: do not modify",
			"{}",
			file.display()
		);
	}
	for directory in ["config", "data", "cache", "state"] {
		assert_eq!(
			std::fs::read_dir(home.path().join(directory).join("timeslay"))
				.unwrap()
				.count(),
			4
		);
	}
}
