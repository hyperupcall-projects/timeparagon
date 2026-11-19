//! Local change observation and refresh cadence, independent of screen rendering.
use super::*;
/// Settings and the refresh generation shared by calendar consumers.
#[derive(Clone, Copy)]
pub(super) struct Session {
	pub config: Signal<timeparagon_core::config::Config>,
	pub revision: Signal<u32>,
}
/// Independently observable outputs of the local change poll.
#[derive(Clone, Copy)]
pub(super) struct SyncState {
	pub google_log: Signal<Vec<calendar::operations::LogEntry>>,
	pub google_sending: Signal<usize>,
	pub local_edits: Signal<u64>,
	pub habit_names: Signal<timeparagon_core::habits::Library>,
	pub nearby_events: Signal<Vec<Event>>,
	pub database_sync_status: Signal<String>,
	pub database_sync_log: Signal<Vec<timeparagon_core::database_sync::LogEntry>>,
}

pub(super) fn use_session() -> Session {
	let config = use_signal(day::load_config);
	let revision = use_signal(|| 0u32);
	Session { config, revision }
}

pub(super) fn use_sync_state() -> SyncState {
	let google_log = use_signal(Vec::<calendar::operations::LogEntry>::new);
	let google_sending = use_signal(|| 0usize);
	let local_edits = use_signal(|| 0u64);
	let habit_names = use_signal(timeparagon_core::habits::Library::default);
	let nearby_events = use_signal(Vec::<calendar::Event>::new);
	let database_sync_status = use_signal(timeparagon_core::database_sync::status);
	let database_sync_log = use_signal(Vec::<timeparagon_core::database_sync::LogEntry>::new);
	use_context_provider(|| calendar_data::Changes(local_edits));
	use_context_provider(|| habits::EventNames(habit_names));
	SyncState {
		google_log,
		google_sending,
		local_edits,
		habit_names,
		nearby_events,
		database_sync_status,
		database_sync_log,
	}
}

#[component]
pub(super) fn RefreshCalendar() -> Element {
	let Session {
		config,
		mut revision,
		..
	} = use_context::<Session>();
	let Navigation {
		mut current_day,
		mut scrolled,
		follow_today,
		..
	} = use_context::<Navigation>();
	let Panels { show_create, .. } = use_context::<Panels>();
	let Selection {
		mut selected_block,
		editing,
		block_writing,
		draft_range,
		..
	} = use_context::<Selection>();
	// Pull again on the same cadence as the iced app, and roll the view over
	// to the new day at the configured boundary unless the user stepped away.
	use_future(move || async move {
		let mut last_refresh = tokio::time::Instant::now();
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(60)).await;
			// Rolling over would take the day out from under an open editor.
			if follow_today()
				&& editing.peek().is_none()
				&& !*show_create.peek()
				&& draft_range.peek().is_none()
				&& !*block_writing.peek()
			{
				let today = day::today(&config.peek().calendar);
				if today != current_day() {
					current_day.set(today);
					selected_block.set(None);
					scrolled.set(false);
				}
			}
			if last_refresh.elapsed() >= CalendarSettings::REFRESH_EVERY {
				last_refresh = tokio::time::Instant::now();
				revision.set(revision() + 1);
			}
		}
	});

	rsx! {}
}

#[component]
pub(super) fn PollLocalChanges() -> Element {
	let Session {
		mut config,
		mut revision,
		..
	} = use_context::<Session>();
	let SyncState {
		mut google_log,
		mut google_sending,
		mut local_edits,
		mut habit_names,
		mut nearby_events,
		mut database_sync_status,
		mut database_sync_log,
		..
	} = use_context::<SyncState>();
	let saved_alarms = use_context::<alarms::Shared>();
	let notes_store = use_context::<notes::NotesStore>();
	// Follow the settings, which another process or another device (through
	// sync) can change. The view is only re-rendered when what it was given
	// differs, so the desktop saving its window size, say, costs one read.
	use_future(move || async move {
		// The local edits are only drawn again when the database says they
		// changed, which is one small query instead of a full read, and the
		// notes only when their directory's fingerprint moved.
		let mut changes: Option<timeparagon_core::local_changes::Counters> = None;
		let mut reader = None::<timeparagon_core::local_changes::Reader>;
		loop {
			let status = timeparagon_core::database_sync::status();
			if status != *database_sync_status.peek() {
				database_sync_status.set(status);
			}
			let new_sync = timeparagon_core::database_sync::log_since(
				database_sync_log
					.peek()
					.last()
					.map_or(0, |entry| entry.sequence),
			);
			if !new_sync.is_empty() {
				database_sync_log.with_mut(|log| {
					log.extend(new_sync);
					let excess = log.len().saturating_sub(100);
					log.drain(..excess);
				});
			}
			let today = day::today(&config.peek().calendar);
			let nearby_dates = vec![
				today - chrono::Duration::days(1),
				today,
				today + chrono::Duration::days(1),
			];
			let hour = config.peek().calendar.day_start_hour();
			let config_before_read = config.peek().clone();
			let result = editor::worker(move || {
				let path = timeparagon_core::task_db::path();
				if reader.is_none() {
					reader = path
						.as_ref()
						.and_then(|path| timeparagon_core::local_changes::Reader::open(path).ok());
				}
				let counters = reader.as_mut().and_then(|reader| reader.read().ok());
				if counters.is_none() {
					reader = None;
				}
				let moved = |part: fn(&timeparagon_core::local_changes::Counters) -> i64| {
					counters.is_none_or(|now| changes.is_none_or(|seen| part(&seen) != part(&now)))
				};
				let nearby = if moved(|c| c.calendar) {
					path.as_ref().map(|path| {
						calendar::repository::read(path, &nearby_dates, hour)
							.map(|read| read.snapshot.events)
					})
				} else {
					None
				};

				let names = moved(|c| c.habits).then(|| habits::read(path).library);
				let reloaded = (changes.is_some() && moved(|c| c.config)).then(day::load_config);
				Ok((reader, counters, nearby, names, reloaded))
			})
			.await;
			let (next_reader, counters, nearby, names, reloaded) = match result {
				Ok(result) => result,
				Err(error) => {
					eprintln!("Could not poll local changes: {error}");
					reader = None;
					tokio::time::sleep(std::time::Duration::from_secs(2)).await;
					continue;
				}
			};
			reader = next_reader;

			let moved = |part: fn(&timeparagon_core::local_changes::Counters) -> i64| {
				counters.is_none_or(|now| changes.is_none_or(|seen| part(&seen) != part(&now)))
			};
			let new = calendar::operations::log_since(
				google_log.peek().last().map_or(0, |entry| entry.sequence),
			);
			if !new.is_empty() {
				// A refused write had its local copy dropped: fetch what Google
				// has so the item shows that again.
				if new.iter().any(|entry| entry.failed) {
					revision.set(revision() + 1);
				}
				google_log.with_mut(|log| log.extend(new));
			}
			let sending = calendar::operations::sending();
			if sending != *google_sending.peek() {
				google_sending.set(sending);
			}
			if moved(|c| c.calendar) {
				local_edits.set(local_edits() + 1);
			}
			if let Some(nearby) = nearby {
				match nearby {
					Ok(events) => nearby_events.set(events),
					Err(error) => eprintln!("Could not read nearby calendar: {error}"),
				}
			}

			if let Some(names) = names
				&& names != *habit_names.peek()
			{
				habit_names.set(names);
			}
			if moved(|c| c.alarms) {
				saved_alarms.reload();
				reminders::republish(&config.peek());
			}
			notes_store.reload_shared().await;
			// Nothing to compare against before the first read, which only
			// repeats the load the view started with.
			// A setting edited while the worker read the database wins over
			// that older read. Leave its counters pending and retry next time.
			let settings_unchanged = *config.peek() == config_before_read;
			if reloaded.is_none() || settings_unchanged {
				changes = counters;
			}
			if let Some(reloaded) = reloaded {
				if !settings_unchanged {
					tokio::time::sleep(std::time::Duration::from_secs(2)).await;
					continue;
				}
				if reloaded.notes != config.peek().notes
					&& let Err(error) = notes_store.configure(&reloaded.notes.directory)
				{
					let mut message = notes_store.error;
					message.set(Some(error));
				}
				if reloaded != *config.peek() {
					config.set(reloaded);
				}
			}
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
		}
	});

	rsx! {}
}

/// The local session's settings, sync outputs and shared stores.
#[component]
pub(super) fn SessionProvider(children: Element) -> Element {
	let session = use_session();
	use_context_provider(|| session);
	use_context_provider(calendar_data::Loader::default);
	let sync = use_sync_state();
	use_context_provider(|| sync);
	alarms::use_shared();
	use_context_provider(notes::NotesStore::new);
	children
}
