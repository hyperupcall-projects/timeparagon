use rusqlite::Connection;
use timeparagon_core::{
	calendar::task_metadata,
	habits::Habit,
	task_db::{Database, SCHEMA_VERSION, TaskData},
};

#[test]
fn an_unsupported_database_is_rejected_without_rewriting_its_data() {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("old.sqlite3");
	let connection = Connection::open(&path).unwrap();
	connection.execute_batch("PRAGMA user_version=1; CREATE TABLE tasks(id INTEGER PRIMARY KEY,metadata TEXT NOT NULL,legacy_key TEXT); INSERT INTO tasks VALUES(100,'keep this data','history');").unwrap();
	assert!(Database::open(&path).is_err());
	assert_eq!(
		connection
			.query_row("SELECT metadata FROM tasks", [], |r| r.get::<_, String>(0))
			.unwrap(),
		"keep this data"
	);
	assert_eq!(
		connection
			.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
			.unwrap(),
		1
	);
}

#[test]
fn a_new_database_and_remote_task_use_the_current_format() {
	let directory = tempfile::tempdir().unwrap();
	let path = directory.path().join("current.sqlite3");
	Database::open(&path).unwrap();
	let connection = Connection::open(&path).unwrap();
	assert_eq!(
		connection
			.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
			.unwrap(),
		SCHEMA_VERSION
	);
	let columns = connection
		.prepare("SELECT name FROM pragma_table_info('tasks')")
		.unwrap()
		.query_map([], |r| r.get::<_, String>(0))
		.unwrap()
		.collect::<Result<Vec<_>, _>>()
		.unwrap();
	assert_eq!(columns, ["id", "metadata"]);
	let data = TaskData {
		id: 100,
		metadata: Default::default(),
	};
	let notes = task_metadata::write("A person's notes", &data).unwrap();
	assert_eq!(task_metadata::read(&notes).unwrap(), Some(data));
	assert_eq!(task_metadata::prose(&notes), "A person's notes");
	assert!(!notes.contains("legacy_key"));
	assert!(task_metadata::read(&(notes + "\ntt-project: 7")).is_err());
}

#[test]
fn retired_habit_shapes_cannot_silently_discard_measurements() {
	let old = r#"{"id":"h","name":"Run","kind":"measurable","target":5,"marks":{"2026-11-08":{"amount":2500}}}"#;
	assert!(serde_json::from_str::<Habit>(old).is_err());
	let old_log = r#"{"id":"h","name":"Run","log":{"2026-11-08":{"happened":["20:05:00"]}}}"#;
	assert!(serde_json::from_str::<Habit>(old_log).is_err());
}
