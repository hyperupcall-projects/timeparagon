//! The single local database: remote task metadata, durable ID/alias allocation,
//! and everything else this device keeps between runs.
//!
//! Tasks, confirmed calendar records and settings (see [`crate::settings`]) have
//! tables of their own, as do habits, brain dumps and task work (see
//! [`crate::relational`]). Other data uses JSON documents named in [`keys`].
//!
use crate::calendar::ids;
use crate::{calendar::Event, timeblocking::Metadata};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
	io,
	path::{Path, PathBuf},
	time::Duration,
};

/// The names of the documents in the database.
pub mod keys {
	pub const HABITS: &str = "habits";
	pub const ALARMS: &str = "alarms";
	pub const CONTACTS: &str = "contacts";
	pub const SYNC_STATE: &str = "calendar_sync_state";
	/// Published by Rust in the Android widget snapshot: see `widget.rs`.
	pub const THEME: &str = "dioxus_theme";
	/// When this database imported `config.toml`: see `settings::import`.
	pub const SETTINGS_IMPORTED_AT: &str = "settings_imported_at";
	/// When a shared section last changed: see `shared_data`.
	pub const SHARED_MODIFIED: &str = "shared_data_modified_ms";
	/// Writes to Google in progress: see `calendar::operations`.
	pub const GOOGLE_DELETES: &str = "google_deletes";
	pub const GOOGLE_SERIES_EDITS: &str = "google_series_edits";

	/// Kept in `caches`: copies of what Google, Mapbox and the People API
	/// hold, fetched again when missing.
	pub const CACHED: [&str; 2] = [CONTACTS, SYNC_STATE];
	/// Kept in `device_state`: what this device keeps about itself and its
	/// own writes in progress. Never replicated, and not rebuilt if lost.
	pub const DEVICE: [&str; 5] = [
		THEME,
		SETTINGS_IMPORTED_AT,
		SHARED_MODIFIED,
		GOOGLE_DELETES,
		GOOGLE_SERIES_EDITS,
	];
}

/// The table a document is kept in: `caches` or `device_state` for the keys
/// listed in [`keys`], `documents` for the rest, which are shared between
/// devices (or were, under an older build).
pub(crate) fn table_of(key: &str) -> &'static str {
	if keys::CACHED.contains(&key) {
		"caches"
	} else if keys::DEVICE.contains(&key) {
		"device_state"
	} else {
		"documents"
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskData {
	pub id: i64,
	pub metadata: Metadata,
}

/// The schema this build sets up, kept in SQLite's `user_version`.
///
/// Schema changes are applied transactionally by [`Database::set_up`].
pub const SCHEMA_VERSION: i64 = 18;

/// Older schemas [`Database::set_up`] can bring up to date.
const UPGRADABLE: std::ops::RangeInclusive<i64> = 2..=17;

fn schema_version(db: &Connection) -> Result<i64, String> {
	db.query_row("PRAGMA user_version", [], |r| r.get(0))
		.map_err(|e| e.to_string())
}

pub struct Database(Connection);
impl Database {
	pub fn open(path: &Path) -> Result<Self, String> {
		if let Some(parent) = path.parent() {
			std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
		}
		let db = Connection::open(path).map_err(|e| e.to_string())?;
		// Holds credentials and personal details. An in-memory database has no file.
		if let Err(error) = crate::fsutil::make_private(path)
			&& error.kind() != io::ErrorKind::NotFound
		{
			return Err(error.to_string());
		}
		db.busy_timeout(Duration::from_secs(10))
			.map_err(|e| e.to_string())?;
		// Per connection, so it cannot be part of the one-off setup below.
		db.execute_batch("PRAGMA foreign_keys = ON;")
			.map_err(|e| e.to_string())?;
		let memory = path == Path::new(":memory:");
		// Setting up the schema is most of the cost of opening, and the apps
		// open the database many times a minute. Once a file has been brought
		// to the current version it is not set up again.
		let version = schema_version(&db)?;
		if version != 0 && version != SCHEMA_VERSION && !UPGRADABLE.contains(&version) {
			return Err(format!(
				"Unsupported database schema {version}; expected {SCHEMA_VERSION}"
			));
		}
		if version == 0
            && db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='tasks') AND (SELECT count(*) FROM pragma_table_info('tasks')) != 2",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
        {
            return Err("Unversioned task databases are no longer supported".into());
        }
		if version == 2 {
			Self::drop_legacy_key(&db).map_err(|e| e.to_string())?;
		}
		// Schema 15, before the setup transaction for the same reason. A value
		// that does not fit leaves the tables as they were: the data still
		// opens, without the new checks.
		if version != 0
			&& version != SCHEMA_VERSION
			&& let Err(error) = crate::relational::make_strict(&db)
		{
			log::warn!("could not make the habit and work tables strict: {error}");
		}
		if memory || version != SCHEMA_VERSION {
			Self::set_up(&db, path, memory)?;
		}
		Ok(Self(db))
	}

	/// Brings the file to [`SCHEMA_VERSION`]. Every step is idempotent, so two
	/// processes doing this at once, or a crash half way, are both harmless.
	fn set_up(db: &Connection, path: &Path, memory: bool) -> Result<(), String> {
		// Immediate, so a second process waits here and then finds each step
		// already done, instead of racing it.
		let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
			.map_err(|e| e.to_string())?;
		// Schema 6: the app's own tables lose the old project name, TimeSlay.
		crate::legacy_name::rename_schema(&tx).map_err(|e| e.to_string())?;
		tx.execute_batch("
            CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS task_aliases (calendar_id TEXT NOT NULL, event_id TEXT NOT NULL, task_id INTEGER NOT NULL REFERENCES tasks(id), PRIMARY KEY (calendar_id, event_id));
            CREATE TABLE IF NOT EXISTS documents (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS device_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS caches (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS google_task_records (id TEXT PRIMARY KEY, event TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS google_entities (calendar_id TEXT NOT NULL, event_id TEXT NOT NULL, event TEXT, acknowledged_at TEXT, observed_at TEXT, sending_since TEXT, PRIMARY KEY (calendar_id, event_id));
            CREATE TABLE IF NOT EXISTS google_id_map (local TEXT PRIMARY KEY, remote TEXT NOT NULL, calendar TEXT NOT NULL);
")
            .map_err(|e| e.to_string())?;
		Self::move_local_documents(&tx).map_err(|e| e.to_string())?;
		// Schema 17: what nothing reads any more. `counter` allocated task IDs
		// before they were random; `google_task_backend_state` copied each
		// task's metadata as Google last confirmed it; the desktop's `routes`
		// cache held the same trips as the `travel.routes` settings.
		tx.execute_batch(
			"DROP TABLE IF EXISTS counter; DROP TABLE IF EXISTS google_task_backend_state;
             DELETE FROM documents WHERE key='routes'; DELETE FROM caches WHERE key='routes';",
		)
		.map_err(|e| e.to_string())?;
		// Schema 5: the settings, which used to be config.toml.
		tx.execute_batch(crate::settings::schema::CREATE)
			.map_err(|e| e.to_string())?;
		// Schema 7 adds the synced metadata table for Markdown notes.
		crate::settings::schema::create_data(&tx).map_err(|e| e.to_string())?;
		// Schema 8: incremental Google sync used to create these on first use.
		crate::calendar::api::incremental::install_schema(&tx).map_err(|e| e.to_string())?;
		Self::remove_write_queue(&tx).map_err(|e| e.to_string())?;
		Self::split_item_keys(&tx).map_err(|e| e.to_string())?;
		// After the tables it watches are rebuilt, which drops their triggers.
		crate::local_changes::install(&tx).map_err(|e| e.to_string())?;
		Self::migrate_calendar(&tx)?;
		crate::local_changes::install_calendar(&tx).map_err(|e| e.to_string())?;
		crate::relational::install(&tx).map_err(|e| e.to_string())?;
		// Schema 18: work sessions keyed by a Google identity move to the
		// task's own ID. After `install`, whose triggers record the move.
		crate::relational::rekey_work_sessions(&tx).map_err(|e| e.to_string())?;
		tx.commit().map_err(|e| e.to_string())?;

		if !memory {
			// Schema 10 is applied here: the retired settings tables move
			// into `settings`, with their sync history.
			crate::database_sync::enroll(path).map_err(|e| e.to_string())?;
			crate::database_sync::install_snapshots(db).map_err(|e| e.to_string())?;
			// After enrolling, so the imported settings are journaled for sync.
			// Only this device's own database takes its config file. A failed
			// import must not keep the database from opening; it is logged
			// and the file left in place.
			if Some(path) == self::path().as_deref()
				&& let Some(file) = crate::config::config_path()
				&& let Err(error) = crate::settings::import_once(db, path, &file)
			{
				log::error!("could not import {}: {error}", file.display());
			}
			// After the import, which may bring in sources under their old IDs.
			crate::legacy_name::rename_source_ids(db).map_err(|e| e.to_string())?;
			// Readers no longer wait on a writer, so a poll on the UI thread
			// cannot stall behind the sync worker's transaction. The mode is
			// kept by the file. It cannot change while another connection is
			// mid-transaction; that is retried on the next open.
			match db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)) {
				Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
				Ok(mode) => {
					log::warn!("database stayed in {mode} mode; will retry");
					return Ok(());
				}
				Err(error) => {
					log::warn!("could not switch the database to WAL: {error}");
					return Ok(());
				}
			}
		}
		db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
			.map_err(|e| e.to_string())?;
		Ok(())
	}

	/// Schema 3: `tasks` loses `legacy_key`. The column is `UNIQUE`, so SQLite
	/// cannot drop it in place and the table is rebuilt, as SQLite documents,
	/// with foreign keys off so `task_aliases` keeps pointing at its rows.
	/// That pragma cannot change inside a transaction, so this runs on its own
	/// before [`Database::set_up`]. Idempotent: a second process finds the
	/// column already gone.
	fn drop_legacy_key(db: &Connection) -> rusqlite::Result<()> {
		db.execute_batch("PRAGMA foreign_keys = OFF;")?;
		let result = (|| {
			let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
			let has_column = tx.query_row(
				"SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='legacy_key')",
				[],
				|r| r.get::<_, bool>(0),
			)?;
			if has_column {
				tx.execute_batch(
					"CREATE TABLE tasks_new (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL);
                     INSERT INTO tasks_new(id,metadata) SELECT id,metadata FROM tasks;
                     DROP TABLE tasks;
                     ALTER TABLE tasks_new RENAME TO tasks;",
				)?;
				let broken = tx
					.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
					.optional()?
					.is_some();
				if broken {
					return Err(rusqlite::Error::InvalidQuery);
				}
			}
			tx.commit()
		})();
		db.execute_batch("PRAGMA foreign_keys = ON;")?;
		result
	}

	/// Schema 4: writes go to Google as they are made instead of through a
	/// durable queue. The queue's tables go, and an item marked as sending
	/// gets `google_entities.sending_since`. Edits still waiting in the old
	/// queue are dropped with it, so those items show what Google has.
	fn remove_write_queue(tx: &Connection) -> rusqlite::Result<()> {
		let has_column = tx.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('google_entities') WHERE name='sending_since')",
			[],
			|r| r.get::<_, bool>(0),
		)?;
		if !has_column {
			tx.execute_batch("ALTER TABLE google_entities ADD COLUMN sending_since TEXT;")?;
		}
		let had_queue = tx
			.query_row(
				"SELECT 1 FROM sqlite_master WHERE type='table' AND name='google_operations'",
				[],
				|_| Ok(()),
			)
			.optional()?
			.is_some();
		if had_queue {
			tx.execute_batch(
                "DELETE FROM google_entities WHERE id IN (SELECT entity FROM google_operations);
                 DELETE FROM google_entities WHERE acknowledged_at IS NULL AND sending_since IS NULL;
                 DROP TRIGGER IF EXISTS timeparagon_change_google_operations_insert;
                 DROP TRIGGER IF EXISTS timeparagon_change_google_operations_update;
                 DROP TRIGGER IF EXISTS timeparagon_change_google_operations_delete;
                 DROP TABLE IF EXISTS google_operation_progress;
                 DROP TABLE IF EXISTS google_operations;
                 DELETE FROM timeparagon_change_counters WHERE name='google.queue';",
            )?;
		}
		Ok(())
	}

	/// Schema 16: documents that are not shared leave `documents` for
	/// `caches` or `device_state` (see [`table_of`]), so `documents` holds
	/// only what devices share. A row already moved wins. Idempotent.
	fn move_local_documents(tx: &Connection) -> rusqlite::Result<()> {
		for (table, moved) in [
			("caches", &keys::CACHED[..]),
			("device_state", &keys::DEVICE[..]),
		] {
			for key in moved {
				tx.execute(
					&format!(
						"INSERT OR IGNORE INTO {table}(key,value) SELECT key,value FROM documents WHERE key=?1"
					),
					[key],
				)?;
				tx.execute("DELETE FROM documents WHERE key=?1", [key])?;
			}
		}
		Ok(())
	}

	/// Schema 14: `task_aliases.source` (`calendar/event`) and
	/// `google_entities.id` (`calendar` newline `event`) become the two
	/// columns `calendar_id` and `event_id`, so neither has to be taken apart
	/// again. SQLite cannot change a primary key in place, so each table is
	/// rebuilt. A value without its separator was never looked up and is not
	/// kept, nor is an alias of a task that is gone. Idempotent: does nothing
	/// once the old column is gone. Runs after [`Self::remove_write_queue`],
	/// which still names `google_entities.id`.
	fn split_item_keys(tx: &Connection) -> rusqlite::Result<()> {
		let has = |table: &str, column: &str| -> rusqlite::Result<bool> {
			tx.query_row(
				"SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
				[table, column],
				|r| r.get(0),
			)
		};
		if has("task_aliases", "source")? {
			tx.execute_batch(
				"CREATE TABLE task_aliases_new (calendar_id TEXT NOT NULL, event_id TEXT NOT NULL, task_id INTEGER NOT NULL REFERENCES tasks(id), PRIMARY KEY (calendar_id, event_id));
                 INSERT INTO task_aliases_new
                   SELECT substr(source,1,instr(source,'/')-1), substr(source,instr(source,'/')+1), task_id
                   FROM task_aliases WHERE instr(source,'/')>0 AND task_id IN (SELECT id FROM tasks);
                 DROP TABLE task_aliases;
                 ALTER TABLE task_aliases_new RENAME TO task_aliases;",
			)?;
		}
		if has("google_entities", "id")? {
			tx.execute_batch(
				"CREATE TABLE google_entities_new (calendar_id TEXT NOT NULL, event_id TEXT NOT NULL, event TEXT, acknowledged_at TEXT, observed_at TEXT, sending_since TEXT, PRIMARY KEY (calendar_id, event_id));
                 INSERT INTO google_entities_new
                   SELECT substr(id,1,instr(id,char(10))-1), substr(id,instr(id,char(10))+1), event, acknowledged_at, observed_at, sending_since
                   FROM google_entities WHERE instr(id,char(10))>0;
                 DROP TABLE google_entities;
                 ALTER TABLE google_entities_new RENAME TO google_entities;",
			)?;
		}
		Ok(())
	}

	/// Schema 9: source rows replace rendered month snapshots. Keep local edits,
	/// stable IDs and stored sleep; convert their old source-row codec once.
	fn migrate_calendar(tx: &Connection) -> Result<(), String> {
		for table in ["google_sync_events", "sleep_nights"] {
			let rows = tx
				.prepare(&format!("SELECT rowid,event FROM {table}"))
				.and_then(|mut q| {
					q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
						.collect::<rusqlite::Result<Vec<_>>>()
				})
				.map_err(|e| e.to_string())?;
			for (id, json) in rows {
				let mut value: serde_json::Value =
					serde_json::from_str(&json).map_err(|e| e.to_string())?;
				if let Some(channels) = value.get("color").and_then(|c| c.as_array()) {
					let channel =
						|i: usize| channels.get(i).and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
					let color = crate::hex::HexColor(iced_core::Color::from_rgba(
						channel(0),
						channel(1),
						channel(2),
						channel(3),
					));
					value["color"] = serde_json::Value::String(color.to_string());
					tx.execute(
						&format!("UPDATE {table} SET event=?2 WHERE rowid=?1"),
						params![id, value.to_string()],
					)
					.map_err(|e| e.to_string())?;
				}
			}
		}
		// Rebuild confirmed task rows from the raw feed, before retiring the old
		// rendered cache. Desired values remain in google_entities throughout.
		if tx
			.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='calendar_months')",
				[],
				|r| r.get::<_, bool>(0),
			)
			.map_err(|e| e.to_string())?
		{
			let tasks = tx
				.prepare("SELECT list_id,entry FROM google_sync_tasks")
				.and_then(|mut q| {
					q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
						.collect::<rusqlite::Result<Vec<_>>>()
				})
				.map_err(|e| e.to_string())?;
			tx.execute("DELETE FROM google_task_records", [])
				.map_err(|e| e.to_string())?;
			for (list, json) in tasks {
				let value: serde_json::Value =
					serde_json::from_str(&json).map_err(|e| e.to_string())?;
				if value["deleted"].as_bool() == Some(true) {
					continue;
				}
				let text = |key: &str| value[key].as_str().unwrap_or_default();
				let day = text("due")
					.get(..10)
					.and_then(|day| day.parse().ok())
					.unwrap_or(crate::timeblocking::UNDATED);
				let notes = text("notes");
				let mut event = Event {
					id: ids::event_id(text("id")),
					calendar_id: ids::calendar_id(&list),
					title: if text("title").is_empty() {
						"(No title)".into()
					} else {
						text("title").to_owned()
					},
					start: crate::calendar::at_clock(day, 0, 0),
					end: crate::calendar::at_clock(day.succ_opt().unwrap_or(day), 0, 0),
					all_day: true,
					color: crate::calendar::colors::TASK,
					completed: text("status") == "completed",
					details: crate::calendar::Details {
						description: crate::calendar::task_metadata::prose(notes).to_owned(),
						link: if text("webViewLink").is_empty() {
							format!("https://tasks.google.com/task/{}", text("id"))
						} else {
							text("webViewLink").to_owned()
						},
						task_updated_at: text("updated")
							.parse::<chrono::DateTime<chrono::Utc>>()
							.ok(),
						task_data: crate::calendar::task_metadata::read(notes)?,
						..Default::default()
					},
				};
				if event.details.task_data.is_none() {
					let mut data = tx
						.query_row(ALIASED, params![event.calendar_id, event.id], read_data)
						.optional()
						.map_err(|e| e.to_string())?;
					if let Some(data) = &mut data {
						let pending:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM google_entities WHERE sending_since IS NOT NULL AND json_extract(event,'$.details.task_data.id')=?1)",[data.id],|r|r.get(0)).map_err(|e|e.to_string())?;
						if pending {
							data.metadata = Default::default();
						}
					}
					event.details.task_data = data;
				}
				tx.execute(
					"INSERT INTO google_task_records VALUES(?1,?2)",
					params![
						event.id,
						serde_json::to_string(&event).map_err(|e| e.to_string())?
					],
				)
				.map_err(|e| e.to_string())?;
			}
			tx.execute_batch(
				"DELETE FROM documents WHERE key='calendar_task_snapshot'; DROP TABLE calendar_months;",
			)
			.map_err(|e| e.to_string())?;
		}

		Ok(())
	}

	pub(crate) fn connection(&mut self) -> &mut Connection {
		&mut self.0
	}

	/// Metadata on its way to Google remains authoritative while Google
	/// refreshes in parallel.
	pub(crate) fn pending_task_data(&self, event: &Event) -> Result<Option<TaskData>, String> {
		let stable = event.details.task_data.as_ref().map(|data| data.id).or(
			crate::calendar::task_metadata::read(&event.details.description)?.map(|data| data.id),
		);
		let json:Option<String>=self.0.query_row("SELECT e.event FROM google_entities e WHERE (json_extract(e.event,'$.id')=?1 OR json_extract(e.event,'$.details.task_data.id')=?2) AND e.sending_since IS NOT NULL",params![event.id,stable],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
		json.map(|json| {
			serde_json::from_str::<Event>(&json)
				.map(|e| e.details.task_data)
				.map_err(|e| e.to_string())
		})
		.transpose()
		.map(Option::flatten)
	}

	/// Adopt authoritative remote data even on a fresh installation.
	pub fn cache_remote(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
		self.save_desired(event, data)
	}

	/// Allocate and record the mapping in one transaction. No ID escapes until commit.
	pub fn import(&mut self, event: &Event) -> Result<TaskData, String> {
		if let Some(data) = &event.details.task_data {
			return Ok(data.clone());
		}
		if let Some(data) = crate::calendar::task_metadata::read(&event.details.description)? {
			self.cache_remote(event, &data)?;
			return Ok(data);
		}
		self.import_inner(event).map_err(|e| e.to_string())
	}
	fn import_inner(&mut self, event: &Event) -> rusqlite::Result<TaskData> {
		let tx = self
			.0
			.transaction_with_behavior(TransactionBehavior::Immediate)?;
		let existing = tx
			.query_row(ALIASED, params![event.calendar_id, event.id], read_data)
			.optional()?;
		let data = if let Some(data) = existing {
			data
		} else {
			let id: i64 = loop {
				let mut bytes = [0u8; 8];
				getrandom::fill(&mut bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
				let candidate = (u64::from_le_bytes(bytes) & ((1u64 << 62) - 1)) as i64;
				if candidate < 100 {
					continue;
				}
				if tx
					.query_row("SELECT 1 FROM tasks WHERE id=?1", [candidate], |_| Ok(()))
					.optional()?
					.is_none()
				{
					break candidate;
				}
			};
			let metadata = Metadata::default();
			let json = serde_json::to_string(&metadata).expect("metadata serializes");
			tx.execute(
				"INSERT INTO tasks(id,metadata) VALUES (?1,?2)",
				params![id, json],
			)?;
			TaskData { id, metadata }
		};
		tx.execute(
			"INSERT INTO task_aliases(calendar_id,event_id,task_id) VALUES (?1,?2,?3) ON CONFLICT DO NOTHING",
			params![event.calendar_id, event.id, data.id],
		)?;
		tx.commit()?;
		Ok(data)
	}

	pub fn lookup(&self, event: &Event) -> Result<Option<TaskData>, String> {
		lookup_in(&self.0, event).map_err(|e| e.to_string())
	}

	/// Records the identity Google confirmed a write under, so a later write
	/// that still names `local_id` (the identity the caller sent, before ID
	/// resolution) reaches it. Leaves the desired metadata alone, so an older
	/// write's answer cannot overwrite a newer local edit.
	pub fn record_remote(&mut self, event: &Event, local_id: &str) -> Result<(), String> {
		let tx = self
			.0
			.transaction_with_behavior(TransactionBehavior::Immediate)
			.map_err(|e| e.to_string())?;
		// A self-mapping also matters: moving back to the original identity
		// must replace its previous destination, rather than leave a cycle.
		if event.is_task() {
			tx.execute("INSERT INTO google_id_map(local,remote,calendar) VALUES(?1,?2,?3) ON CONFLICT(local) DO UPDATE SET remote=excluded.remote,calendar=excluded.calendar", params![ids::bare_task(local_id),ids::bare_task(&event.id),event.calendar_id]).map_err(|e| e.to_string())?;
		}
		tx.commit().map_err(|e| e.to_string())
	}

	/// Read metadata for a remote write without adopting its older snapshot as
	/// the user's desired state. Allocate only if it has never had metadata.
	pub fn import_remote(&mut self, event: &Event) -> Result<TaskData, String> {
		if let Some(data) = &event.details.task_data {
			return Ok(data.clone());
		}
		if let Some(data) = crate::calendar::task_metadata::read(&event.details.description)? {
			return Ok(data);
		}
		self.import_inner(event).map_err(|e| e.to_string())
	}

	/// Save the user's desired metadata and its Google identity locally.
	pub fn save_desired(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
		(|| -> rusqlite::Result<()> {
            let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let json = serde_json::to_string(&data.metadata).expect("metadata serializes");
            tx.execute("INSERT INTO tasks (id,metadata) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata", params![data.id,json])?;
            tx.execute(ALIAS, params![event.calendar_id, event.id, data.id])?;
            tx.commit()
        })().map_err(|e| e.to_string())
	}
}
/// The task an event's Google identity names, if it has been given an ID.
pub(crate) fn lookup_in(
	connection: &Connection,
	event: &Event,
) -> rusqlite::Result<Option<TaskData>> {
	connection
		.query_row(ALIASED, params![event.calendar_id, event.id], read_data)
		.optional()
}

/// A task's ID and metadata by any Google identity it has had.
const ALIASED: &str = "SELECT t.id,t.metadata FROM tasks t JOIN task_aliases a ON a.task_id=t.id WHERE a.calendar_id=?1 AND a.event_id=?2";

/// Records that a Google identity names a task, replacing what it named before.
pub(crate) const ALIAS: &str = "INSERT INTO task_aliases(calendar_id,event_id,task_id) VALUES(?1,?2,?3) ON CONFLICT(calendar_id,event_id) DO UPDATE SET task_id=excluded.task_id";

fn read_data(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskData> {
	let json: String = row.get(1)?;
	let metadata = serde_json::from_str(&json).map_err(|e| {
		rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
	})?;
	Ok(TaskData {
		id: row.get(0)?,
		metadata,
	})
}

/// The database on this device.
pub fn path() -> Option<PathBuf> {
	Some(
		crate::config::project_dirs()?
			.data_local_dir()
			.join("timeparagon.sqlite3"),
	)
}

pub fn open() -> Result<Database, String> {
	Database::open(&path().ok_or("Cannot locate local data directory")?)
}

/// A document, or `None` when nothing has saved it yet. Contents that do not
/// parse are an error, so a caller can decline to save over them.
pub fn read_document<T: DeserializeOwned>(path: &Path, key: &str) -> io::Result<Option<T>> {
	let database = Database::open(path).map_err(io::Error::other)?;
	if crate::relational::handles(key) {
		return crate::relational::read(&database.0, path, key)?
			.map(serde_json::from_value)
			.transpose()
			.map_err(io::Error::other);
	}
	let table = table_of(key);
	let text: Option<String> = database
		.0
		.query_row(
			&format!("SELECT value FROM {table} WHERE key=?1"),
			[key],
			|row| row.get(0),
		)
		.optional()
		.map_err(io::Error::other)?;
	let result = text
		.as_ref()
		.map(|text| {
			serde_json::from_str(text)
				.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
		})
		.transpose()?;
	if table == "documents" {
		crate::database_sync::projection::remember(path, key, text);
	}
	Ok(result)
}

pub fn write_document<T: Serialize + ?Sized>(path: &Path, key: &str, value: &T) -> io::Result<()> {
	let json = serde_json::to_string(value)
		.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
	write_document_json(path, key, &json)
}

/// [`write_document`] for a value that is already serialized.
pub fn write_document_json(path: &Path, key: &str, json: &str) -> io::Result<()> {
	change_document(path, key, Some(json.to_owned()))
}

pub fn remove_document(path: &Path, key: &str) -> io::Result<()> {
	change_document(path, key, None)
}

fn change_document(path: &Path, key: &str, desired: Option<String>) -> io::Result<()> {
	let mut database = Database::open(path).map_err(io::Error::other)?;
	let tx = database
		.0
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(io::Error::other)?;
	if crate::relational::handles(key) {
		let value = desired
			.as_deref()
			.map(serde_json::from_str)
			.transpose()
			.map_err(io::Error::other)?
			.unwrap_or_else(|| serde_json::json!({}));
		let rows = crate::relational::save(&tx, path, key, &value)?;
		tx.commit().map_err(io::Error::other)?;
		return crate::relational::remember(path, key, rows);
	}
	// Nothing else writes these, so there is no other device's edit to merge.
	let table = table_of(key);
	if table != "documents" {
		match desired {
			Some(value) => tx.execute(
				&format!("INSERT INTO {table}(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value"),
				params![key, value],
			),
			None => tx.execute(&format!("DELETE FROM {table} WHERE key=?1"), [key]),
		}
		.map_err(io::Error::other)?;
		return tx.commit().map_err(io::Error::other);
	}
	let value = crate::database_sync::projection::prepare(path, &tx, key, &desired)?;
	match value {
		Some(value) => {
			tx.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(io::Error::other)?;
		}
		None => {
			tx.execute("DELETE FROM documents WHERE key=?1", [key])
				.map_err(io::Error::other)?;
		}
	}
	tx.commit().map_err(io::Error::other)?;
	crate::database_sync::projection::remember(path, key, desired);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	fn path(label: &str) -> std::path::PathBuf {
		std::env::temp_dir()
			.join(format!(
				"tt-db-{label}-{}-{}",
				std::process::id(),
				std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_nanos()
			))
			.join("timeparagon.sqlite3")
	}
	fn event(id: &str) -> Event {
		let mut event = crate::calendar::sample_snapshot().events.remove(0);
		event.id = ids::event_id(id);
		event.calendar_id = "tasks:test".into();
		event.details = Default::default();
		event
	}

	#[test]
	fn remote_confirmation_keeps_newer_desired_metadata_and_maps_the_local_identity() {
		let mut db = Database::open(Path::new(":memory:")).unwrap();
		let local = event("original-local");
		let mut wanted = TaskData {
			id: 123,
			metadata: Metadata::default(),
		};
		wanted.metadata.minutes = 45;
		db.save_desired(&local, &wanted).unwrap();
		let mut confirmed = wanted.clone();
		confirmed.metadata.minutes = 15;
		let mut remote = event("latest-remote");
		remote.details.description = crate::calendar::task_metadata::write("", &confirmed).unwrap();
		// Reading an older write's metadata must not adopt it as a new edit.
		assert_eq!(db.import_remote(&remote).unwrap(), confirmed);
		db.record_remote(&remote, &local.id).unwrap();
		assert_eq!(db.lookup(&local).unwrap(), Some(wanted));
		let mapping: (String, String) =
			db.0.query_row(
				"SELECT remote,calendar FROM google_id_map WHERE local='original-local'",
				[],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.unwrap();
		assert_eq!(mapping, ("latest-remote".into(), "tasks:test".into()));
		// Returning to the original ID retires the earlier mapping.
		db.record_remote(&local, &local.id).unwrap();
		let remote: String =
			db.0.query_row(
				"SELECT remote FROM google_id_map WHERE local='original-local'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(remote, "original-local");
	}

	#[test]
	fn schema_6_adds_notes_metadata_and_sync_triggers() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("db.sqlite3");
		drop(Database::open(&path).unwrap());
		let db = Connection::open(&path).unwrap();
		db.execute_batch("DROP TABLE notes; PRAGMA user_version=6;")
			.unwrap();
		drop(db);
		let mut db = Database::open(&path).unwrap();
		let connection = db.connection();
		assert_eq!(schema_version(connection).unwrap(), SCHEMA_VERSION);
		connection
			.execute("INSERT INTO notes(id) VALUES('n1')", [])
			.unwrap();
		let pending: i64 = connection
			.query_row(
				"SELECT count(*) FROM timeparagon_sync_edits WHERE document='notes:[\"n1\"]'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(pending, 1);
		assert_eq!(
			crate::local_changes::read_from(connection).unwrap().notes,
			1
		);
	}

	#[test]
	fn schema_7_installs_lazy_tables_and_triggers_on_open() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("db.sqlite3");
		drop(Database::open(&path).unwrap());
		let db = Connection::open(&path).unwrap();
		let objects = |db: &Connection| -> Vec<(String, String, Option<String>)> {
			db.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
				.unwrap()
				.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
				.unwrap()
				.collect::<Result<_, _>>()
				.unwrap()
		};
		let fresh_schema = objects(&db);
		// A version 7 device that has never used incremental sync or stored
		// sleep. Keep notes, which version 7 already created.
		db.execute_batch(
			"DROP TABLE google_sync_meta; DROP TABLE google_sync_calendars;
            DROP TABLE google_sync_windows; DROP TABLE google_sync_events;
            DROP TABLE google_sync_task_lists; DROP TABLE google_sync_tasks;
            DROP TABLE sleep_nights; PRAGMA user_version=7;",
		)
		.unwrap();
		drop(db);
		let mut database = Database::open(&path).unwrap();
		let db = database.connection();
		assert_eq!(schema_version(db).unwrap(), SCHEMA_VERSION);
		assert_eq!(objects(db), fresh_schema);
		db.execute(
			"INSERT INTO sleep_nights VALUES('app','n','2030-11-04','a','b','{}')",
			[],
		)
		.unwrap();
		assert_eq!(crate::local_changes::read_from(db).unwrap().sleep, 1);
		let edits: i64 = db
			.query_row(
				"SELECT count(*) FROM timeparagon_sync_edits WHERE document GLOB 'sleep_nights:*'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(edits, 0, "nights stay on the device");
	}

	#[test]
	fn schema_7_keeps_existing_lazy_data_and_reopening_is_a_noop() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("db.sqlite3");
		drop(Database::open(&path).unwrap());
		let db = Connection::open(&path).unwrap();
		db.execute_batch(
			"INSERT INTO google_sync_meta VALUES('account','me');
            INSERT INTO google_sync_calendars VALUES('app','token');
            INSERT INTO google_sync_windows VALUES('a','b');
            INSERT INTO google_sync_events VALUES('app','event',NULL,'a','b',0,'{}');
            INSERT INTO google_sync_task_lists VALUES('list','watermark');
            INSERT INTO google_sync_tasks VALUES('list','task','{}');
            INSERT INTO sleep_nights VALUES('app','night','2030-11-04','a','b','{}');
            INSERT INTO notes VALUES('note','rose','[]');
            INSERT INTO documents VALUES('unknown','preserve me');
            PRAGMA user_version=7;",
		)
		.unwrap();
		drop(db);
		let before = crate::database_migration::inspect(&path).unwrap();
		let history = crate::database_sync::history(&path).unwrap();
		for _ in 0..2 {
			let mut database = Database::open(&path).unwrap();
			assert_eq!(
				schema_version(database.connection()).unwrap(),
				SCHEMA_VERSION
			);
			assert_eq!(crate::database_migration::inspect(&path).unwrap(), before);
			assert_eq!(crate::database_sync::history(&path).unwrap(), history);
			let db = database.connection();
			let token: String = db
				.query_row("SELECT sync_token FROM google_sync_calendars", [], |r| {
					r.get(0)
				})
				.unwrap();
			assert_eq!(token, "token");
			let color: String = db
				.query_row("SELECT color FROM notes", [], |r| r.get(0))
				.unwrap();
			assert_eq!(color, "rose");
		}
	}

	#[test]
	fn schema_8_failure_rolls_back_and_can_be_retried() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("db.sqlite3");
		drop(Database::open(&path).unwrap());
		let db = Connection::open(&path).unwrap();
		// A conflicting object makes index creation fail after setup has
		// already tried to create another table. Neither change may stick.
		db.execute_batch(
			"DROP TABLE google_sync_meta; DROP TABLE google_sync_events;
            CREATE VIEW google_sync_events AS SELECT 1;
            PRAGMA user_version=7;",
		)
		.unwrap();
		assert!(Database::open(&path).is_err());
		assert_eq!(schema_version(&db).unwrap(), 7);
		let created: bool = db
			.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='google_sync_meta')",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert!(!created);
		db.execute_batch("DROP VIEW google_sync_events").unwrap();
		drop(db);
		let mut database = Database::open(&path).unwrap();
		assert_eq!(
			schema_version(database.connection()).unwrap(),
			SCHEMA_VERSION
		);
	}

	#[test]
	fn documents_round_trip_and_bad_contents_are_an_error() {
		let path = path("documents");
		assert_eq!(read_document::<String>(&path, "a").unwrap(), None);
		write_document(&path, "a", "first").unwrap();
		write_document(&path, "a", "second").unwrap();
		assert_eq!(
			read_document::<String>(&path, "a").unwrap().as_deref(),
			Some("second")
		);
		write_document_json(&path, "a", "not json").unwrap();
		assert_eq!(
			read_document::<String>(&path, "a").unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		remove_document(&path, "a").unwrap();
		assert_eq!(read_document::<String>(&path, "a").unwrap(), None);
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}
	#[test]
	fn schema_2_is_upgraded_keeping_its_tasks() {
		let path = path("schema-2-upgrade");
		Database::open(&path).unwrap();
		{
			// Put the file back as schema 2 left it: `tasks` with a unique
			// `legacy_key`, an alias pointing at it, and the old write queue.
			let db = Connection::open(&path).unwrap();
			db.execute_batch(
                "DROP TABLE task_aliases; DROP TABLE tasks; DROP TABLE google_entities;
                 CREATE TABLE tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL, legacy_key TEXT UNIQUE);
                 CREATE TABLE task_aliases (source TEXT PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id));
                 CREATE TABLE google_entities (id TEXT PRIMARY KEY, event TEXT, acknowledged_at TEXT, observed_at TEXT);
                 INSERT INTO tasks VALUES (100,'{}','old'),(101,'{}',NULL);
                 INSERT INTO task_aliases VALUES ('tasks:list/task:a',100),('cal/b',101),('no separator',101);
                 CREATE TABLE google_operations (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, payload TEXT NOT NULL, entity TEXT NOT NULL, created_at TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, lease_until INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1, dead_at TEXT);
                 CREATE TABLE google_operation_progress (operation_id INTEGER PRIMARY KEY REFERENCES google_operations(id) ON DELETE CASCADE, data TEXT NOT NULL);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
		}
		let mut db = Database::open(&path).unwrap();
		let connection = db.connection();
		assert_eq!(schema_version(connection).unwrap(), SCHEMA_VERSION);
		let columns: Vec<String> = connection
			.prepare("SELECT name FROM pragma_table_info('tasks')")
			.unwrap()
			.query_map([], |r| r.get(0))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(columns, ["id", "metadata"]);
		let aliases: Vec<(String, String, i64)> = connection
			.prepare(
				"SELECT a.calendar_id,a.event_id,t.id FROM task_aliases a JOIN tasks t ON t.id=a.task_id ORDER BY a.calendar_id",
			)
			.unwrap()
			.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		// Split at the separator; one without it could never be looked up.
		assert_eq!(
			aliases,
			[
				("cal".into(), "b".into(), 101),
				("tasks:list".into(), "task:a".into(), 100)
			]
		);
		let foreign_keys: bool = connection
			.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
			.unwrap();
		assert!(foreign_keys);
		drop(db);
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}
	#[test]
	fn upgrading_drops_the_old_write_queue_and_its_unsent_edits() {
		let path = path("queue-upgrade");
		Database::open(&path).unwrap();
		{
			// Put the file back as schema 3 left it: a queue holding one
			// unsent edit, and an edit Google had already confirmed.
			let db = Connection::open(&path).unwrap();
			db.execute_batch(
                "DROP TABLE google_entities;
                 CREATE TABLE google_entities (id TEXT PRIMARY KEY, event TEXT, acknowledged_at TEXT, observed_at TEXT);
                 CREATE TABLE google_operations (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, payload TEXT NOT NULL, entity TEXT NOT NULL, created_at TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, lease_until INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1, dead_at TEXT);
                 CREATE TABLE google_operation_progress (operation_id INTEGER PRIMARY KEY REFERENCES google_operations(id) ON DELETE CASCADE, data TEXT NOT NULL);
                 CREATE TRIGGER timeparagon_change_google_operations_insert AFTER INSERT ON google_operations BEGIN SELECT 1; END;
                 INSERT INTO google_operations(kind,payload,entity,created_at) VALUES('Edit event','{}','cal
unsent','now');
                 INSERT INTO google_entities(id,event,acknowledged_at) VALUES('cal
unsent','{}',NULL);
                 INSERT INTO google_entities(id,event,acknowledged_at) VALUES('cal
confirmed','{}','2026-01-01T00:00:00+00:00');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
		}
		let mut db = Database::open(&path).unwrap();
		let connection = db.connection();
		assert_eq!(schema_version(connection).unwrap(), SCHEMA_VERSION);
		let tables: i64 = connection
			.query_row(
				"SELECT count(*) FROM sqlite_master WHERE name LIKE 'google_operation%'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(tables, 0);
		let kept: Vec<String> = connection
			.prepare(
				"SELECT calendar_id||'/'||event_id FROM google_entities WHERE sending_since IS NULL",
			)
			.unwrap()
			.query_map([], |r| r.get(0))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(kept, ["cal/confirmed"]);
		// The overlay's change triggers were rebuilt with its table.
		let triggers: i64 = connection
			.query_row(
				"SELECT count(*) FROM sqlite_master WHERE type='trigger' AND tbl_name='google_entities'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(triggers, 3);
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}
	#[cfg(unix)]
	#[test]
	fn the_database_is_readable_only_by_this_user() {
		use std::os::unix::fs::PermissionsExt;
		let path = path("private");
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(&path, b"").unwrap();
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
		Database::open(&path).unwrap();
		let mode = std::fs::metadata(&path).unwrap().permissions().mode();
		assert_eq!(mode & 0o777, 0o600);
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}
	#[test]
	fn allocator_process() {
		let Some(path) = std::env::var_os("TT_TEST_DATABASE") else {
			return;
		};
		let worker = std::env::var("TT_TEST_WORKER").unwrap();
		let mut db = Database::open(Path::new(&path)).unwrap();
		for i in 0..20 {
			db.import(&event(&format!("{worker}-{i}"))).unwrap();
			db.import(&event("shared-task")).unwrap();
		}
	}
	#[test]
	fn concurrent_processes_get_unique_ids_and_one_shared_mapping() {
		let path = path("concurrent");
		let children: Vec<_> = (0..4)
			.map(|worker| {
				std::process::Command::new(std::env::current_exe().unwrap())
					.args([
						"--exact",
						"task_db::tests::allocator_process",
						"--nocapture",
					])
					.env("TT_TEST_DATABASE", &path)
					.env("TT_TEST_WORKER", worker.to_string())
					.stdout(std::process::Stdio::null())
					.spawn()
					.unwrap()
			})
			.collect();
		for mut child in children {
			assert!(child.wait().unwrap().success());
		}
		let db = Database::open(&path).unwrap();
		let (count, min, max): (i64, i64, i64) =
			db.0.query_row("SELECT count(*),min(id),max(id) FROM tasks", [], |row| {
				Ok((row.get(0)?, row.get(1)?, row.get(2)?))
			})
			.unwrap();
		assert_eq!(count, 81);
		assert!(min >= 100 && max < (1_i64 << 62));
		let aliases: i64 =
			db.0.query_row("SELECT count(*) FROM task_aliases", [], |r| r.get(0))
				.unwrap();
		assert_eq!(aliases, 81, "all processes reuse the same shared task");
		std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
	}
}

#[cfg(test)]
mod local_document_tests {
	use super::*;

	#[test]
	fn schema_15_moves_caches_and_device_state_out_of_the_shared_documents() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("test.sqlite3");
		Database::open(&path).unwrap();
		let moved = keys::CACHED
			.iter()
			.chain(&keys::DEVICE)
			.copied()
			.collect::<Vec<_>>();
		{
			// Put the rows back where schema 15 kept them.
			let db = Connection::open(&path).unwrap();
			for key in &moved {
				db.execute(
					"INSERT INTO documents VALUES(?1,?2)",
					params![key, format!("{{\"was\":\"{key}\"}}")],
				)
				.unwrap();
			}
			db.execute(
				"INSERT INTO documents VALUES('alarms','{\"alarms\":[]}')",
				[],
			)
			.unwrap();
			db.execute_batch("PRAGMA user_version=15;").unwrap();
		}
		Database::open(&path).unwrap();
		for key in &moved {
			let value: serde_json::Value = read_document(&path, key).unwrap().unwrap();
			assert_eq!(value["was"], *key);
		}
		let db = Connection::open(&path).unwrap();
		let left: Vec<String> = db
			.prepare("SELECT key FROM documents ORDER BY key")
			.unwrap()
			.query_map([], |r| r.get(0))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(left, ["alarms"]);
		let count = |table: &str| -> i64 {
			db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
				.unwrap()
		};
		assert_eq!(count("caches"), keys::CACHED.len() as i64);
		assert_eq!(count("device_state"), keys::DEVICE.len() as i64);
		// Watchers see a change to a moved document as before.
		let before = crate::local_changes::read(&path).unwrap();
		write_document_json(&path, keys::SYNC_STATE, "{}").unwrap();
		remove_document(&path, keys::GOOGLE_DELETES).unwrap();
		let after = crate::local_changes::read(&path).unwrap();
		assert_eq!(after.calendar, before.calendar + 2);
		assert!(
			read_document::<serde_json::Value>(&path, keys::GOOGLE_DELETES)
				.unwrap()
				.is_none()
		);
	}
}

#[cfg(test)]
mod retired_table_tests {
	use super::*;

	#[test]
	fn schema_16_drops_what_nothing_reads() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("test.sqlite3");
		Database::open(&path).unwrap();
		{
			let db = Connection::open(&path).unwrap();
			db.execute_batch(
				"CREATE TABLE counter (value INTEGER);
                 CREATE TABLE google_task_backend_state (task_id INTEGER PRIMARY KEY, data TEXT NOT NULL);
                 INSERT INTO google_task_backend_state VALUES (123, '{}');
                 INSERT INTO caches VALUES ('routes', '[]');
                 INSERT INTO caches VALUES ('contacts', '{}');
                 PRAGMA user_version=16;",
			)
			.unwrap();
		}
		Database::open(&path).unwrap();
		let db = Connection::open(&path).unwrap();
		let tables: i64 = db
			.query_row(
				"SELECT count(*) FROM sqlite_master WHERE name IN ('counter','google_task_backend_state')",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(tables, 0);
		let cached: Vec<String> = db
			.prepare("SELECT key FROM caches")
			.unwrap()
			.query_map([], |r| r.get(0))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(cached, ["contacts"]);
	}
}

#[cfg(test)]
mod calendar_migration_tests {
	use super::*;

	#[test]
	fn schema_8_retires_rendered_caches_and_preserves_sources_and_pending_edits() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut db = Database::open(&path).unwrap();
		let mut snapshot = crate::calendar::sample_snapshot();
		snapshot.events.truncate(1);
		let confirmed = &snapshot.events[0];
		let mut old = serde_json::to_value(confirmed).unwrap();
		old["color"] = serde_json::json!([
			confirmed.color.r,
			confirmed.color.g,
			confirmed.color.b,
			confirmed.color.a
		]);
		let mut desired = confirmed.clone();
		desired.title = "Keep this pending edit".into();
		db.connection().execute_batch("CREATE TABLE calendar_months(month TEXT PRIMARY KEY,events TEXT NOT NULL); INSERT INTO calendar_months VALUES('2026-10','{\"events\":[]}'); INSERT INTO documents VALUES('calendar_task_snapshot','true'); INSERT INTO google_sync_calendars VALUES('primary','keep-token'); PRAGMA user_version=8;").unwrap();
		db.connection()
			.execute(
				"INSERT INTO google_sync_events VALUES(?1,?2,NULL,'a','b',0,?3)",
				params![confirmed.calendar_id, confirmed.id, old.to_string()],
			)
			.unwrap();
		db.connection()
			.execute(
				"INSERT INTO google_entities(calendar_id,event_id,event) VALUES(?1,?2,?3)",
				params![
					desired.calendar_id,
					desired.id,
					serde_json::to_string(&desired).unwrap()
				],
			)
			.unwrap();
		db.connection().execute("INSERT INTO google_sync_tasks VALUES('list','task',?1)",[serde_json::json!({"id":"task","title":"Confirmed task","status":"needsAction"}).to_string()]).unwrap();
		db.connection()
			.execute(
				"INSERT INTO google_id_map VALUES('local','remote','primary')",
				[],
			)
			.unwrap();
		drop(db);
		let mut upgraded = Database::open(&path).unwrap();
		let tx = upgraded.connection();
		assert_eq!(schema_version(tx).unwrap(), SCHEMA_VERSION);
		assert!(
			!tx.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='calendar_months')",
				[],
				|r| r.get::<_, bool>(0)
			)
			.unwrap()
		);
		assert_eq!(
			tx.query_row("SELECT sync_token FROM google_sync_calendars", [], |r| {
				r.get::<_, String>(0)
			})
			.unwrap(),
			"keep-token"
		);
		let json: String = tx
			.query_row("SELECT event FROM google_sync_events", [], |r| r.get(0))
			.unwrap();
		assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), *confirmed);
		let task: String = tx
			.query_row("SELECT event FROM google_task_records", [], |r| r.get(0))
			.unwrap();
		assert_eq!(
			serde_json::from_str::<Event>(&task).unwrap().title,
			"Confirmed task"
		);
		assert_eq!(
			tx.query_row("SELECT count(*) FROM google_id_map", [], |r| r
				.get::<_, i64>(0))
				.unwrap(),
			1
		);
		assert_eq!(
			crate::calendar::repository::event(&path, &desired.calendar_id, &desired.id)
				.unwrap()
				.unwrap()
				.title,
			desired.title
		);
	}

	#[test]
	fn a_failed_calendar_upgrade_rolls_back_the_schema_and_source_rows() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		drop(Database::open(&path).unwrap());
		let db = Connection::open(&path).unwrap();
		db.execute_batch("CREATE TABLE calendar_months(month TEXT PRIMARY KEY,events TEXT NOT NULL);
            INSERT INTO google_sync_events VALUES('c','e',NULL,'a','b',0,'{\"color\":[1,0,0,1]}');
            CREATE TRIGGER fail_calendar_upgrade BEFORE UPDATE ON google_sync_events BEGIN SELECT RAISE(ABORT,'test failure'); END;
            PRAGMA user_version=8;").unwrap();
		drop(db);
		assert!(Database::open(&path).is_err());
		let db = Connection::open(&path).unwrap();
		assert_eq!(schema_version(&db).unwrap(), 8);
		assert!(
			db.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='calendar_months')",
				[],
				|r| r.get::<_, bool>(0)
			)
			.unwrap()
		);
		assert_eq!(
			db.query_row(
				"SELECT json_type(event,'$.color') FROM google_sync_events",
				[],
				|r| r.get::<_, String>(0)
			)
			.unwrap(),
			"array"
		);
	}
}
