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
			happened: vec![
				LogEntry {
					at: Some(NaiveTime::from_hms_opt(10, 20, 30).unwrap()),
					amount: Some(1250),
					note: "hello".into(),
				},
				LogEntry::default(),
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
		raw.query_row("SELECT count(*) FROM habit_log_entries", [], |r| r
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
