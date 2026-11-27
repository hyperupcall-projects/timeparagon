use super::*;
use crate::habits::{self, DayLog, Frequency, Habit, Library, LogEntry, Mark, Measure, Section};
use crate::task_db;
use chrono::{DateTime, NaiveDate, NaiveTime};
use serde_json::json;

fn day(n: u32) -> NaiveDate {
	NaiveDate::from_ymd_opt(2026, 11, n).unwrap()
}
fn database() -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("test.db");
	task_db::Database::open(&path).unwrap();
	(dir, path)
}
fn db(path: &Path) -> Connection {
	let db = Connection::open(path).unwrap();
	db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
	db
}
fn library() -> Library {
	let mut a = Habit::new("a", "Run 🏃");
	a.frequency = Frequency::Weekdays {
		days: [true, false, true, false, true, false, true],
	};
	a.created = Some(day(1));
	a.kind = habits::Kind::No;
	a.icons = habits::Icons::Faces;
	a.section = Some("group".into());
	a.measure = Some(Measure {
		unit: "miles".into(),
		target: Some(1.25),
		days: 7,
	});
	a.area = Some(u64::MAX);
	a.distraction = true;
	a.archived = true;
	a.skip_days = true;
	a.question_marks = true;
	a.question = "Did you?".into();
	a.notes = "a note".into();
	a.marks.insert(day(1), Mark::Yes);
	a.marks.insert(day(2), Mark::No);
	a.marks.insert(day(3), Mark::Skip);
	a.marks.insert(day(4), Mark::Reps(3));
	a.values.insert(day(5), 0);
	a.resisted.insert(day(5), 0);
	a.values.insert(day(1), 1250);
	a.log.insert(
		day(1),
		DayLog {
			// In order of time, as `Habit::set_log` keeps them.
			happened: vec![
				LogEntry::default(),
				LogEntry {
					at: Some(NaiveTime::from_hms_opt(10, 20, 30).unwrap()),
					amount: Some(1250),
					note: "hello".into(),
				},
			],
			resisted: vec![LogEntry::new(None, Some(2.0))],
		},
	);
	a.log.insert(day(6), DayLog::default());
	let mut b = Habit::new("b", "Read");
	b.frequency = Frequency::TimesPerDay { times: 3 };
	Library {
		habits: vec![b, a],
		sections: vec![Section {
			id: "group".into(),
			name: "Health".into(),
			collapsed: true,
		}],
		reversed: true,
		hide_completed: true,
		sort: habits::SortKey::Score,
		score_bucket: habits::Bucket::Year,
		history_bucket: habits::Bucket::Quarter,
		..Library::default()
	}
}
fn work() -> crate::task_work::WorkLog {
	let mut w = crate::task_work::WorkLog::default();
	let at = DateTime::from_timestamp(1000, 123456789).unwrap();
	w.start("task:123".into(), "Title".into(), Some("block".into()), at);
	for _ in 0..2 {
		w.distractions.push(crate::task_work::Distraction {
			block_key: "block".into(),
			minutes: 5,
			recorded_at: at,
		});
	}
	w
}
#[test]
fn schema_10_documents_migrate_atomically_and_reopen_without_journaling_collections() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("old.db");
	let raw = db(&path);
	raw.execute_batch(
		"CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); PRAGMA user_version=10;",
	)
	.unwrap();
	let l = library();
	let w = work();
	let brain = json!({"entries":[{"date":"2026-11-18","text":""},{"date":"2026-11-01","text":"hello\nworld"}]});
	for (key, v) in [
		("habits", json!(l)),
		("shared.task_work", json!(w)),
		("shared.brain_dump", brain.clone()),
	] {
		raw.execute(
			"INSERT INTO documents VALUES(?1,?2)",
			params![key, v.to_string()],
		)
		.unwrap();
	}
	drop(raw);
	task_db::Database::open(&path).unwrap();
	let actual: Library = task_db::read_document(&path, "habits").unwrap().unwrap();
	assert_eq!(actual, l);
	let actual: crate::task_work::WorkLog = task_db::read_document(&path, "shared.task_work")
		.unwrap()
		.unwrap();
	assert_eq!(actual, w);
	let actual: Value = task_db::read_document(&path, "shared.brain_dump")
		.unwrap()
		.unwrap();
	assert_eq!(actual, brain);
	let raw = db(&path);
	assert_eq!(
		raw.query_row(
			"SELECT count(*) FROM timeparagon_relational_legacy",
			[],
			|r| r.get::<_, i64>(0)
		)
		.unwrap(),
		3
	);
	assert_eq!(
		raw.query_row(
			"SELECT count(*) FROM documents WHERE key IN ('habits','shared.task_work','shared.brain_dump')",
			[],
			|r| r.get::<_, i64>(0)
		)
		.unwrap(),
		0
	);
	assert_eq!(raw.query_row("SELECT count(*) FROM timeparagon_sync_edits WHERE document IN ('habits','shared.task_work','shared.brain_dump')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
	assert_eq!(
		raw.query_row("SELECT count(*) FROM habit_entries", [], |r| r
			.get::<_, i64>(0))
			.unwrap(),
		3
	);
	assert!(
		raw.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
			.optional()
			.unwrap()
			.is_none()
	);
	let pending = sync::pending(&raw).unwrap();
	task_db::Database::open(&path).unwrap();
	crate::database_sync::enroll(&path).unwrap();
	assert_eq!(
		pending.iter().map(|c| &c.revision).collect::<Vec<_>>(),
		sync::pending(&raw)
			.unwrap()
			.iter()
			.map(|c| &c.revision)
			.collect::<Vec<_>>()
	);
}
#[test]
fn invalid_or_duplicate_legacy_records_leave_the_original_and_schema_intact() {
	for value in [
		"not json",
		r#"{"habits":[{"id":"x","name":"a"},{"id":"x","name":"b"}]}"#,
	] {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("old.db");
		let raw = db(&path);
		raw.execute_batch(
			"CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); PRAGMA user_version=10;",
		)
		.unwrap();
		raw.execute("INSERT INTO documents VALUES('habits',?1)", [value])
			.unwrap();
		assert!(task_db::Database::open(&path).is_err());
		assert_eq!(
			raw.query_row("SELECT value FROM documents", [], |r| r.get::<_, String>(0))
				.unwrap(),
			value
		);
		assert_eq!(
			raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
				.unwrap(),
			10
		);
		assert!(
			!raw.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='habit_days')",
				[],
				|r| r.get::<_, bool>(0)
			)
			.unwrap()
		);
	}
}
#[test]
fn ticking_one_day_journals_one_small_row_and_repeated_ticks_coalesce() {
	let (_dir, path) = database();
	let (storage, mut l) = habits::store::Storage::load(Some(path.clone())).unwrap();
	let mut h = Habit::new("a", "A");
	for n in 0..2000 {
		h.marks
			.insert(day(1) - chrono::Duration::days(n), Mark::Yes);
	}
	l.habits.push(h);
	storage.save(&l).unwrap();
	let raw = db(&path);
	sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
	let counters = crate::local_changes::read(&path).unwrap();
	for mark in [Mark::No, Mark::Yes, Mark::Skip] {
		l.habits[0].marks.insert(day(2), mark);
		storage.save(&l).unwrap();
	}
	let pending = sync::pending(&raw).unwrap();
	assert_eq!(pending.len(), 1);
	assert_eq!(TABLES[pending[0].table].name, "habit_days");
	assert!(pending[0].row.as_ref().unwrap().len() < 10);
	assert!(crate::local_changes::read(&path).unwrap().habits > counters.habits);
	let total: i64 = raw
		.query_row(
			"SELECT count(*) FROM timeparagon_sync_edits WHERE document='habits'",
			[],
			|r| r.get(0),
		)
		.unwrap();
	assert_eq!(total, 0);
	storage.save(&l).unwrap();
	assert_eq!(
		sync::pending(&raw).unwrap()[0].revision,
		pending[0].revision
	);
}
#[test]
fn independent_habit_views_preserve_unseen_habits_and_dates() {
	let (_dir, path) = database();
	let (first, mut a) = habits::store::Storage::load(Some(path.clone())).unwrap();
	a.habits.push(Habit::new("a", "A"));
	first.save(&a).unwrap();
	let (second, mut b) = habits::store::Storage::load(Some(path.clone())).unwrap();
	b.habits[0].marks.insert(day(1), Mark::Yes);
	b.habits.push(Habit::new("b", "B"));
	second.save(&b).unwrap();
	a.habits[0].marks.insert(day(2), Mark::No);
	first.save(&a).unwrap();
	let actual = habits::store::Storage::load(Some(path)).unwrap().1;
	assert_eq!(actual.habits.len(), 2);
	assert_eq!(actual.habits[0].marks.len(), 2);
}
#[test]
fn stale_habit_or_day_changes_do_not_resurrect_a_deleted_habit() {
	let (_dir, path) = database();
	let (first, mut a) = habits::store::Storage::load(Some(path.clone())).unwrap();
	a.habits.push(Habit::new("a", "A"));
	first.save(&a).unwrap();
	let (second, mut b) = habits::store::Storage::load(Some(path.clone())).unwrap();
	b.habits.clear();
	second.save(&b).unwrap();
	a.habits[0].name = "Renamed".into();
	a.habits[0].marks.insert(day(1), Mark::Yes);
	first.save(&a).unwrap();
	assert!(
		habits::store::Storage::load(Some(path))
			.unwrap()
			.1
			.habits
			.is_empty()
	);
}
#[test]
fn acknowledgement_does_not_clear_a_newer_local_edit() {
	let (_dir, path) = database();
	task_db::write_document(
		&path,
		"shared.brain_dump",
		&json!({"entries":[{"date":"2026-11-18","text":"one"}]}),
	)
	.unwrap();
	let raw = db(&path);
	let sending = sync::pending(&raw).unwrap();
	raw.execute("UPDATE brain_dump_entries SET text='two'", [])
		.unwrap();
	sync::acknowledge(&raw, &sending).unwrap();
	let pending = sync::pending(&raw).unwrap();
	assert_eq!(pending.len(), 1);
	assert_ne!(pending[0].revision, sending[0].revision);
	assert_eq!(pending[0].row.as_ref().unwrap()[1], "two");
}
#[test]
fn primary_key_changes_record_the_old_deletion_and_new_identity() {
	let (_dir, path) = database();
	task_db::write_document(
		&path,
		"shared.brain_dump",
		&json!({"entries":[{"date":"2026-11-18","text":"one"}]}),
	)
	.unwrap();
	let raw = db(&path);
	sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
	raw.execute("UPDATE brain_dump_entries SET date='2026-11-19'", [])
		.unwrap();
	let pending = sync::pending(&raw).unwrap();
	assert_eq!(pending.len(), 2);
	assert_eq!(pending.iter().filter(|c| c.deleted).count(), 1);
}

fn runtime() -> tokio::runtime::Runtime {
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap()
}
async fn server(path: &Path) -> (turso::Database, turso::Connection) {
	let db = turso::Builder::new_local(path.to_str().unwrap())
		.build()
		.await
		.unwrap();
	let conn = db.connect().unwrap();
	// The sync engine records every write on its connection; match it.
	conn.execute("PRAGMA capture_data_changes_conn('full,turso_cdc')", ())
		.await
		.unwrap();
	sync::install(&conn).await.unwrap();
	conn.execute(
		"CREATE TABLE timeparagon_revisions_v1(id TEXT PRIMARY KEY,payload TEXT NOT NULL)",
		(),
	)
	.await
	.unwrap();
	(db, conn)
}
async fn send(conn: &turso::Connection, path: &Path) {
	let raw = db(path);
	let pending = sync::pending(&raw).unwrap();
	sync::send(conn, &pending).await.unwrap();
	sync::acknowledge(&raw, &pending).unwrap();
}
async fn pull(conn: &turso::Connection, path: &Path) {
	let raw = db(path);
	let changes = sync::receive(conn, &raw).await.unwrap();
	sync::apply(&raw, &changes).unwrap();
}
#[test]
fn native_tables_round_trip_offline_edits_deletions_and_all_scalar_types() {
	runtime().block_on(async {
		let (d1, p1) = database();
		let (_d2, p2) = database();
		let (_server, conn) = server(&d1.path().join("remote.db")).await;
		let l = library();
		task_db::write_document(&p1, "habits", &l).unwrap();
		task_db::write_document(&p1, "shared.task_work", &work()).unwrap();
		task_db::write_document(
			&p1,
			"shared.brain_dump",
			&json!({"entries":[{"date":"2026-11-18","text":"hello"}]}),
		)
		.unwrap();
		send(&conn, &p1).await;
		pull(&conn, &p2).await;
		assert_eq!(
			task_db::read_document::<Library>(&p2, "habits")
				.unwrap()
				.unwrap(),
			l
		);
		assert_eq!(
			task_db::read_document::<crate::task_work::WorkLog>(&p2, "shared.task_work")
				.unwrap()
				.unwrap(),
			work()
		);
		assert!(sync::pending(&db(&p2)).unwrap().is_empty());
		let (s1, mut a) = habits::store::Storage::load(Some(p1.clone())).unwrap();
		let (s2, mut b) = habits::store::Storage::load(Some(p2.clone())).unwrap();
		a.habits[0].marks.insert(day(10), Mark::Yes);
		b.habits[1].marks.insert(day(11), Mark::No);
		s1.save(&a).unwrap();
		s2.save(&b).unwrap();
		send(&conn, &p1).await;
		send(&conn, &p2).await;
		pull(&conn, &p1).await;
		pull(&conn, &p2).await;
		let a = habits::store::Storage::load(Some(p1.clone())).unwrap().1;
		let b = habits::store::Storage::load(Some(p2.clone())).unwrap().1;
		assert_eq!(a, b);
		assert_eq!(a.habits[0].marks.get(&day(10)), Some(&Mark::Yes));
		assert_eq!(a.habits[1].marks.get(&day(11)), Some(&Mark::No));
		task_db::remove_document(&p1, "shared.brain_dump").unwrap();
		send(&conn, &p1).await;
		pull(&conn, &p2).await;
		assert!(
			task_db::read_document::<crate::brain_dump::Dumps>(&p2, "shared.brain_dump")
				.unwrap()
				.unwrap()
				.is_empty()
		);
		let raw = db(&p2);
		let changes = sync::receive(&conn, &raw).await.unwrap();
		assert!(changes.is_empty());
	});
}
#[test]
fn stopping_a_work_session_beside_others_syncs_the_longer_row() {
	runtime().block_on(async {
		let (d1, p1) = database();
		let (_d2, p2) = database();
		let (_server, conn) = server(&d1.path().join("remote.db")).await;
		let block = format!(
			"{}@group.calendar.google.com/{}",
			"a".repeat(64),
			"b".repeat(32)
		);
		let mut w = crate::task_work::WorkLog::default();
		for i in 0..7 {
			let at = DateTime::from_timestamp(1000 + i * 60, 598590497).unwrap();
			w.start(
				format!("task:{i}"),
				format!("Study session {i}"),
				Some(block.clone()),
				at,
			);
		}
		task_db::write_document(&p1, "shared.task_work", &w).unwrap();
		send(&conn, &p1).await;
		// Stopping the active session lengthens a row that shares its page.
		w.stop_all(DateTime::from_timestamp(2000, 235675389).unwrap());
		task_db::write_document(&p1, "shared.task_work", &w).unwrap();
		// A retry after a lost acknowledgement resends identical rows.
		let raw = db(&p1);
		let pending = sync::pending(&raw).unwrap();
		assert_eq!(pending.len(), 1);
		sync::send(&conn, &pending).await.unwrap();
		sync::send(&conn, &pending).await.unwrap();
		sync::acknowledge(&raw, &pending).unwrap();
		pull(&conn, &p2).await;
		assert_eq!(
			task_db::read_document::<crate::task_work::WorkLog>(&p2, "shared.task_work")
				.unwrap()
				.unwrap(),
			w
		);
	});
}
#[test]
fn old_cloud_history_is_imported_once_then_ignored_after_cutover() {
	runtime().block_on(async {
		let (dir, path) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		let edit = crate::database_sync::model::Edit {
			id: "old".into(),
			document: "shared.brain_dump".into(),
			after: Some(json!({"entries":[{"date":"2026-11-18","text":"cloud"}]}).to_string()),
			clock: 1,
			..Default::default()
		};
		conn.execute(
			"INSERT INTO timeparagon_revisions_v1 VALUES(?1,?2)",
			(edit.id.clone(), serde_json::to_string(&edit).unwrap()),
		)
		.await
		.unwrap();
		assert!(sync::bootstrap(&conn, &path, "test-server").await.unwrap());
		send(&conn, &path).await;
		assert_eq!(
			task_db::read_document::<crate::brain_dump::Dumps>(&path, "shared.brain_dump")
				.unwrap()
				.unwrap()
				.get(day(18)),
			Some("cloud")
		);
		task_db::remove_document(&path, "shared.brain_dump").unwrap();
		send(&conn, &path).await;
		let (_dir, new_path) = database();
		assert!(
			!sync::bootstrap(&conn, &new_path, "test-server")
				.await
				.unwrap()
		);
		pull(&conn, &new_path).await;
		assert!(
			task_db::read_document::<crate::brain_dump::Dumps>(&new_path, "shared.brain_dump")
				.unwrap()
				.unwrap()
				.is_empty()
		);
	});
}
#[test]
fn late_upgrades_keep_new_edits_without_restoring_old_cloud_rows() {
	runtime().block_on(async {
        let (dir,path)=database();let (_server,conn)=server(&dir.path().join("remote.db")).await;
        conn.execute("INSERT INTO timeparagon_relational_cutover VALUES(1)",()).await.unwrap();
        let old=json!({"entries":[{"date":"2026-11-01","text":"old"},{"date":"2026-11-02","text":"also old"}]});
        let oldpath=dir.path().join("late.db");let raw=db(&oldpath);raw.execute_batch("CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); PRAGMA user_version=10;").unwrap();raw.execute("INSERT INTO documents VALUES('shared.brain_dump',?1)",[old.to_string()]).unwrap();drop(raw);
        let mut old=old;task_db::Database::open(&oldpath).unwrap();let _: Value=task_db::read_document(&oldpath,"shared.brain_dump").unwrap().unwrap();old["entries"][0]["text"]=json!("new local edit");task_db::write_document(&oldpath,"shared.brain_dump",&old).unwrap();
        sync::bootstrap(&conn,&oldpath,"test-server").await.unwrap();send(&conn,&oldpath).await;pull(&conn,&path).await;
        let dumps=task_db::read_document::<crate::brain_dump::Dumps>(&path,"shared.brain_dump").unwrap().unwrap();assert_eq!(dumps.entries.len(),1);assert_eq!(dumps.get(day(1)),Some("new local edit"));
    });
}

#[test]
fn malformed_native_rows_roll_back_the_entire_received_batch() {
	let (_dir, path) = database();
	let raw = db(&path);
	let good = sync::Change {
		table: 6,
		key: json!(["2026-11-18"]).to_string(),
		revision: "good".into(),
		deleted: false,
		row: Some(vec![json!("2026-11-18"), json!("text"), json!(0)]),
	};
	let bad = sync::Change {
		table: 6,
		key: json!(["invalid-date"]).to_string(),
		revision: "bad".into(),
		deleted: false,
		row: Some(vec![json!("invalid-date"), json!("text"), json!(1)]),
	};
	assert!(sync::apply(&raw, &[good, bad]).is_err());
	assert!(all(&raw, Some("shared.brain_dump")).unwrap().is_empty());
	assert!(sync::pending(&raw).unwrap().is_empty());
	assert_eq!(
		raw.query_row(
			"SELECT projecting FROM timeparagon_relational_control",
			[],
			|r| r.get::<_, i64>(0)
		)
		.unwrap(),
		0
	);
}
#[test]
fn concurrent_changes_to_the_same_date_use_the_last_native_write() {
	runtime().block_on(async {
		let (dir, a) = database();
		let (_dir, b) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		let doc = |text| json!({"entries":[{"date":"2026-11-18","text":text}]});
		task_db::write_document(&a, "shared.brain_dump", &doc("original")).unwrap();
		send(&conn, &a).await;
		pull(&conn, &b).await;
		task_db::write_document(&a, "shared.brain_dump", &doc("from A")).unwrap();
		task_db::write_document(&b, "shared.brain_dump", &doc("from B")).unwrap();
		send(&conn, &a).await;
		send(&conn, &b).await;
		pull(&conn, &a).await;
		pull(&conn, &b).await;
		for path in [&a, &b] {
			assert_eq!(
				task_db::read_document::<crate::brain_dump::Dumps>(path, "shared.brain_dump")
					.unwrap()
					.unwrap()
					.get(day(18)),
				Some("from B")
			);
		}
	});
}
#[test]
fn a_received_habit_deletion_wins_over_unseen_child_rows() {
	runtime().block_on(async {
		let (dir, a) = database();
		let (_dir, b) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		let mut l = Library::default();
		l.habits.push(Habit::new("a", "A"));
		task_db::write_document(&a, "habits", &l).unwrap();
		send(&conn, &a).await;
		pull(&conn, &b).await;
		let (sa, mut la) = habits::store::Storage::load(Some(a.clone())).unwrap();
		let (sb, mut lb) = habits::store::Storage::load(Some(b.clone())).unwrap();
		la.habits.clear();
		sa.save(&la).unwrap();
		lb.habits[0].marks.insert(day(18), Mark::Yes);
		sb.save(&lb).unwrap();
		send(&conn, &a).await;
		send(&conn, &b).await;
		pull(&conn, &a).await;
		pull(&conn, &b).await;
		for path in [&a, &b] {
			assert!(
				habits::store::Storage::load(Some(path.clone()))
					.unwrap()
					.1
					.habits
					.is_empty()
			);
			assert!(sync::pending(&db(path)).unwrap().is_empty());
		}
	});
}
#[test]
fn a_new_server_is_seeded_with_previously_acknowledged_rows() {
	runtime().block_on(async {
		let (dir, path) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		task_db::write_document(&path, "shared.task_work", &work()).unwrap();
		let raw = db(&path);
		sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
		assert!(sync::pending(&raw).unwrap().is_empty());
		assert!(sync::bootstrap(&conn, &path, "new-server").await.unwrap());
		send(&conn, &path).await;
		let (_dir, b) = database();
		pull(&conn, &b).await;
		assert_eq!(
			task_db::read_document::<crate::task_work::WorkLog>(&b, "shared.task_work")
				.unwrap()
				.unwrap(),
			work()
		);
	});
}

fn strict(raw: &Connection, table: &str) -> bool {
	raw.query_row(
		"SELECT strict FROM pragma_table_list WHERE schema='main' AND name=?1",
		[table],
		|r| r.get(0),
	)
	.unwrap()
}
/// Rebuilds every relational table as schema 14 made it: not `STRICT`, with
/// `BOOLEAN` columns and without the newer `CHECK`s.
fn loosen(raw: &Connection) {
	raw.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
	for table in TABLES {
		let columns = table
			.columns
			.iter()
			.enumerate()
			.map(|(i, (name, ty))| {
				let ty = ty.split(" CHECK").next().unwrap();
				format!(
					"{name} {ty}{}",
					if i < table.keys { " NOT NULL" } else { "" }
				)
			})
			.collect::<Vec<_>>()
			.join(",");
		let keys = table.columns[..table.keys]
			.iter()
			.map(|c| c.0)
			.collect::<Vec<_>>()
			.join(",");
		let names = table.names();
		raw.execute_batch(&format!(
			"CREATE TABLE loose({columns},PRIMARY KEY({keys}));
			 INSERT INTO loose SELECT {names} FROM {name}; DROP TABLE {name};
			 ALTER TABLE loose RENAME TO {name};",
			name = table.name
		))
		.unwrap();
		assert!(!strict(raw, table.name));
	}
	raw.execute_batch("PRAGMA foreign_keys=ON; PRAGMA user_version=14;")
		.unwrap();
}
#[test]
fn the_tables_refuse_values_of_the_wrong_type_or_shape() {
	let (_dir, path) = database();
	task_db::write_document(&path, "habits", &library()).unwrap();
	let raw = db(&path);
	for table in TABLES {
		assert!(strict(&raw, table.name), "{}", table.name);
	}
	for (statement, why) in [
		("UPDATE habits SET color='red'", "a color is a number"),
		("UPDATE habits SET archived=2", "a boolean is 0 or 1"),
		(
			"UPDATE habits SET measure_unit=NULL WHERE measure_present",
			"a measure has a unit",
		),
		(
			"UPDATE habits SET measure_days=7 WHERE NOT measure_present",
			"no measure, no days",
		),
		(
			"UPDATE habits SET frequency_days=2 WHERE frequency_kind='times_per_day'",
			"times a day has no days",
		),
		(
			"UPDATE habits SET frequency_kind='interval' WHERE frequency_kind='times_per_day'",
			"an interval has its days",
		),
		(
			"UPDATE habit_days SET repetitions=NULL WHERE mark='reps'",
			"reps have a count",
		),
		(
			"UPDATE habit_days SET repetitions=2 WHERE mark='yes'",
			"only reps have a count",
		),
	] {
		assert!(raw.execute(statement, []).is_err(), "{why}");
	}
	assert_eq!(
		task_db::read_document::<Library>(&path, "habits")
			.unwrap()
			.unwrap(),
		library()
	);
}
#[test]
fn schema_14_tables_are_rebuilt_strict_with_their_rows_triggers_and_outbox() {
	let (_dir, path) = database();
	task_db::write_document(&path, "habits", &library()).unwrap();
	task_db::write_document(&path, "shared.task_work", &work()).unwrap();
	let raw = db(&path);
	let outbox = |raw: &Connection| -> Vec<(String, String, String)> {
		raw.prepare(
			"SELECT table_name,row_key,revision FROM timeparagon_relational_state ORDER BY 1,2",
		)
		.unwrap()
		.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
		.unwrap()
		.collect::<rusqlite::Result<_>>()
		.unwrap()
	};
	let objects = |raw: &Connection| -> i64 {
		raw.query_row(
			"SELECT count(*) FROM sqlite_master WHERE type IN ('trigger','index') AND sql IS NOT NULL",
			[],
			|r| r.get(0),
		)
		.unwrap()
	};
	let (rows, pending, triggers) = (all(&raw, None).unwrap(), outbox(&raw), objects(&raw));
	loosen(&raw);
	assert!(objects(&raw) < triggers);
	drop(raw);
	task_db::Database::open(&path).unwrap();
	let raw = db(&path);
	for table in TABLES {
		assert!(strict(&raw, table.name), "{}", table.name);
	}
	assert_eq!(all(&raw, None).unwrap(), rows);
	assert_eq!(outbox(&raw), pending);
	assert_eq!(objects(&raw), triggers);
	assert!(
		raw.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
			.optional()
			.unwrap()
			.is_none()
	);
	// The triggers are back: a change is recorded for sync.
	sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
	raw.execute("UPDATE habits SET name='renamed' WHERE id='b'", [])
		.unwrap();
	assert_eq!(sync::pending(&raw).unwrap().len(), 1);
}
#[test]
fn a_value_that_does_not_fit_leaves_the_tables_loose_and_the_file_opening() {
	let (_dir, path) = database();
	task_db::write_document(&path, "habits", &library()).unwrap();
	let raw = db(&path);
	loosen(&raw);
	raw.execute("UPDATE habits SET archived=7 WHERE id='b'", [])
		.unwrap();
	drop(raw);
	task_db::Database::open(&path).unwrap();
	let raw = db(&path);
	for table in TABLES {
		assert!(!strict(&raw, table.name), "{}", table.name);
	}
	assert_eq!(
		raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
			.unwrap(),
		task_db::SCHEMA_VERSION
	);
}
/// A day with one logged entry, as each device saves it.
fn logged(path: &Path, at: (u32, u32), note: &str) {
	let (storage, mut library) = habits::store::Storage::load(Some(path.to_owned())).unwrap();
	let habit = library.habits.iter_mut().find(|h| h.id == "a").unwrap();
	let mut entries = habit
		.log
		.get(&day(18))
		.cloned()
		.unwrap_or_default()
		.happened;
	entries.push(LogEntry::new(NaiveTime::from_hms_opt(at.0, at.1, 0), None).with_note(note));
	habit
		.marks
		.insert(day(18), Mark::Reps(entries.len() as u32));
	habit.set_log(day(18), &entries, &[]);
	storage.save(&library).unwrap();
}
fn notes(path: &Path) -> Vec<String> {
	let library = habits::store::Storage::load(Some(path.to_owned()))
		.unwrap()
		.1;
	let habit = library.habits.iter().find(|h| h.id == "a").unwrap();
	habit.log[&day(18)]
		.happened
		.iter()
		.map(|e| e.note.clone())
		.collect()
}
#[test]
fn log_entries_added_on_two_devices_at_once_are_both_kept() {
	runtime().block_on(async {
		let (dir, a) = database();
		let (_dir, b) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		let mut l = Library::default();
		l.habits.push(Habit::new("a", "Water"));
		task_db::write_document(&a, "habits", &l).unwrap();
		logged(&a, (8, 0), "breakfast");
		send(&conn, &a).await;
		pull(&conn, &b).await;
		// Both append the second entry of the day while offline.
		logged(&a, (12, 0), "lunch on A");
		logged(&b, (13, 0), "lunch on B");
		send(&conn, &a).await;
		send(&conn, &b).await;
		pull(&conn, &a).await;
		pull(&conn, &b).await;
		for path in [&a, &b] {
			assert_eq!(notes(path), ["breakfast", "lunch on A", "lunch on B"]);
		}
		// Removing the first entry leaves the others' rows alone.
		let raw = db(&a);
		sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
		let (storage, mut library) = habits::store::Storage::load(Some(a.clone())).unwrap();
		let habit = &mut library.habits[0];
		let rest = habit.log[&day(18)].happened[1..].to_vec();
		habit.set_log(day(18), &rest, &[]);
		storage.save(&library).unwrap();
		let pending = sync::pending(&raw).unwrap();
		let entries = pending
			.iter()
			.filter(|c| TABLES[c.table].name == "habit_entries");
		assert_eq!(entries.clone().count(), 1);
		assert!(entries.clone().all(|c| c.deleted));
	});
}
#[test]
fn schema_11_log_entries_move_to_their_new_keys_with_their_sync_state() {
	let (_dir, path) = database();
	task_db::write_document(&path, "habits", &library()).unwrap();
	let raw = db(&path);
	sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
	let expected = all(&raw, Some("habits")).unwrap();
	let tokens = |raw: &Connection| -> BTreeMap<String, (String, i64)> {
		raw.prepare("SELECT row_key,revision,dirty FROM timeparagon_relational_state WHERE table_name='habit_entries'")
			.unwrap()
			.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
			.unwrap()
			.collect::<rusqlite::Result<_>>()
			.unwrap()
	};
	let synced = tokens(&raw);
	assert_eq!(synced.len(), 3);
	// Rebuild the schema 11 table from the current rows, as an old build left it.
	raw.execute_batch(
		"UPDATE timeparagon_relational_control SET projecting=1;
		 CREATE TABLE habit_log_entries(habit_id TEXT NOT NULL,date TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('happened','resisted')),position INTEGER NOT NULL,at TEXT,amount INTEGER,note TEXT NOT NULL,PRIMARY KEY(habit_id,date,kind,position),FOREIGN KEY(habit_id,date) REFERENCES habit_days(habit_id,date) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED);
		 CREATE TEMP TABLE dense AS SELECT *, row_number() OVER (PARTITION BY habit_id,date,kind ORDER BY at,position) - 1 AS dense FROM habit_entries;
		 INSERT INTO habit_log_entries SELECT habit_id,date,kind,dense,at,amount,note FROM dense;
		 UPDATE timeparagon_relational_state SET table_name='habit_log_entries',
		   row_key=(SELECT json_array(e.habit_id,e.date,e.kind,e.dense) FROM dense e WHERE json_array(e.habit_id,e.date,e.id)=row_key)
		   WHERE table_name='habit_entries';
		 DELETE FROM habit_entries;
		 UPDATE timeparagon_relational_control SET projecting=0;",
	)
	.unwrap();
	// One entry was edited and another deleted before the upgrade, unsent.
	raw.execute_batch(
		"UPDATE habit_log_entries SET note='edited' WHERE kind='resisted';
		 UPDATE timeparagon_relational_state SET dirty=1,revision='edited' WHERE row_key LIKE '%\"resisted\",0]';
		 INSERT INTO timeparagon_relational_state VALUES('habit_log_entries','[\"a\",\"2026-11-01\",\"happened\",9]','gone',1,1);
		 PRAGMA user_version=11;",
	)
	.unwrap();
	drop(raw);
	task_db::Database::open(&path).unwrap();
	let raw = db(&path);
	let exists = |name: &str| -> bool {
		raw.query_row(
			"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
			[name],
			|r| r.get(0),
		)
		.unwrap()
	};
	assert!(!exists("habit_log_entries"));
	let mut expected = expected;
	for ((i, _), row) in expected.iter_mut() {
		if TABLES[*i].name == "habit_entries" && row[3] == "resisted" {
			row[7] = json!("edited");
		}
	}
	let actual = all(&raw, Some("habits")).unwrap();
	// The edited entry's content, and so its key, changed; the others kept theirs.
	assert_eq!(actual.len(), expected.len());
	let notes = |rows: &Rows| {
		let mut notes = rows
			.iter()
			.filter(|((i, _), _)| TABLES[*i].name == "habit_entries")
			.map(|(_, r)| r[7].to_string())
			.collect::<Vec<_>>();
		notes.sort();
		notes
	};
	assert_eq!(notes(&actual), notes(&expected));
	let migrated = tokens(&raw);
	assert_eq!(migrated.len(), 3);
	for (key, (revision, dirty)) in &migrated {
		match synced.get(key) {
			Some(token) => assert_eq!(&(revision.clone(), *dirty), token),
			None => assert_eq!((revision.as_str(), *dirty), ("edited", 1)),
		}
	}
	assert_eq!(
		raw.query_row(
			"SELECT count(*) FROM timeparagon_relational_state WHERE table_name='habit_log_entries'",
			[],
			|r| r.get::<_, i64>(0)
		)
		.unwrap(),
		0
	);
	// Reading and saving the migrated habits changes nothing.
	let (storage, library) = habits::store::Storage::load(Some(path.clone())).unwrap();
	storage.save(&library).unwrap();
	assert_eq!(sync::pending(&raw).unwrap().len(), 1);
}
#[test]
fn confirmed_rows_the_server_lacks_are_sent_again() {
	runtime().block_on(async {
		let (dir, a) = database();
		let (_server, conn) = server(&dir.path().join("remote.db")).await;
		task_db::write_document(&a, "shared.task_work", &work()).unwrap();
		// Confirmed locally, as rows moved by an upgrade are, but never sent.
		let raw = db(&a);
		sync::acknowledge(&raw, &sync::pending(&raw).unwrap()).unwrap();
		pull(&conn, &a).await;
		let resent = sync::pending(&raw).unwrap();
		assert_eq!(
			resent.len(),
			work().sessions.len() + work().distractions.len()
		);
		send(&conn, &a).await;
		pull(&conn, &a).await;
		assert!(sync::pending(&raw).unwrap().is_empty());
		let (_dir, b) = database();
		pull(&conn, &b).await;
		assert_eq!(
			task_db::read_document::<crate::task_work::WorkLog>(&b, "shared.task_work")
				.unwrap()
				.unwrap(),
			work()
		);
	});
}

#[test]
fn config_saves_use_the_supplied_baseline_even_after_another_reader_refreshes() {
	let (_dir, path) = database();
	let mut initial = crate::config::Config::default();
	initial.brain_dump.set(day(1), "one".into());
	crate::settings::save(&path, &crate::config::Config::default(), &initial).unwrap();
	let stale = crate::settings::load::<crate::config::Config>(&path, |_| {})
		.unwrap()
		.value;
	let mut other = stale.clone();
	other.brain_dump.set(day(2), "two".into());
	crate::settings::save(&path, &stale, &other).unwrap();
	let _refreshed = crate::settings::load::<crate::config::Config>(&path, |_| {}).unwrap();
	let mut desired = stale.clone();
	desired.brain_dump.set(day(1), "edited".into());
	crate::settings::save(&path, &stale, &desired).unwrap();
	let actual = crate::settings::load::<crate::config::Config>(&path, |_| {})
		.unwrap()
		.value;
	assert_eq!(actual.brain_dump.get(day(1)), Some("edited"));
	assert_eq!(actual.brain_dump.get(day(2)), Some("two"));
}

#[test]
fn merged_work_rows_choose_the_latest_session_and_distraction_by_time() {
	let mut w = work();
	let mut later = w.sessions[0].clone();
	later.started_at += chrono::Duration::seconds(10);
	later.task_key = "later".into();
	w.sessions.insert(0, later);
	let mut later = w.distractions[0].clone();
	later.recorded_at += chrono::Duration::seconds(10);
	later.minutes = 30;
	w.distractions.insert(0, later);
	let rows = codec::encode("shared.task_work", &json!(w)).unwrap();
	let mut actual: crate::task_work::WorkLog =
		serde_json::from_value(codec::decode("shared.task_work", &rows).unwrap()).unwrap();
	assert_eq!(actual.active().unwrap().task_key, "later");
	actual.undo_distraction("block");
	assert!(actual.distractions.iter().all(|d| d.minutes == 5));
}

#[test]
fn retiring_documents_with_old_triggers_does_not_journal_a_user_deletion() {
	runtime().block_on(async {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("old.db");let raw=db(&path);
        raw.execute_batch("CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE timeparagon_sync_control(singleton INTEGER PRIMARY KEY,projecting INTEGER NOT NULL,generation INTEGER NOT NULL);
            INSERT INTO timeparagon_sync_control VALUES(1,0,0);
            CREATE TABLE timeparagon_sync_edits(id TEXT PRIMARY KEY,document TEXT NOT NULL,before_json TEXT,after_json TEXT,clock INTEGER NOT NULL,uploaded INTEGER NOT NULL DEFAULT 0);
            CREATE TRIGGER timeparagon_sync_delete AFTER DELETE ON documents WHEN OLD.key='habits' AND (SELECT projecting FROM timeparagon_sync_control)=0 BEGIN
            INSERT INTO timeparagon_sync_edits VALUES('migration-must-not-delete',OLD.key,OLD.value,NULL,2,0); END;
            PRAGMA user_version=10;").unwrap();
        let l=library();raw.execute("INSERT INTO documents VALUES('habits',?1)",[json!(l).to_string()]).unwrap();
        raw.execute("INSERT INTO timeparagon_sync_edits VALUES('original','habits',NULL,?1,1,1)",[json!(l).to_string()]).unwrap();
        task_db::Database::open(&path).unwrap();
        assert_eq!(raw.query_row("SELECT count(*) FROM timeparagon_sync_edits WHERE document='habits' AND after_json IS NULL",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let (_server,conn)=server(&dir.path().join("remote.db")).await;sync::bootstrap(&conn,&path,"test-server").await.unwrap();
        assert_eq!(habits::store::Storage::load(Some(path)).unwrap().1,l);
    });
}
