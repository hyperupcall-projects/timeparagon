//! Cached day selection, derived models, links, scrolling and platform publication.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct DayData {
	pub loaded: Resource<Result<(Vec<NaiveDate>, calendar::Snapshot), String>>,
	pub disk_view: calendar_data::CachedView,
	pub snapshot: Memo<Option<calendar::Snapshot>>,
	#[cfg(target_os = "android")]
	pub model: Memo<Option<DayModel>>,
	#[cfg(target_os = "android")]
	pub travel: Memo<Vec<day::TravelBox>>,
	pub display: Memo<(Option<DayModel>, Vec<day::TravelBox>)>,
	pub calendar_sources: Memo<Vec<calendar::Calendar>>,
}

pub(super) fn use_day_data() -> DayData {
	let Session { config, revision } = use_context::<Session>();
	let SyncState { habit_names, .. } = use_context::<SyncState>();
	let Navigation {
		current_day, page, ..
	} = use_context::<Navigation>();
	let Panels { show_agenda, .. } = use_context::<Panels>();
	let Selection {
		selected_block,
		search_state,
		..
	} = use_context::<Selection>();
	let settings = config.read().calendar.clone();
	let (data, disk_view) = calendar_data::use_calendar(
		vec![current_day()],
		settings.clone(),
		revision,
		page() != Page::Plan && !show_agenda(),
	);
	let snapshot = use_memo(move || {
		let resource = data.read();
		let requested_day = current_day();
		let mut snapshot = snapshot_for_day(requested_day, resource.as_ref(), || {
			disk_view.read().clone()
		});
		drop(resource);
		if let Some(snapshot) = &mut snapshot {
			let names = habit_names.read();
			for event in &mut snapshot.events {
				names.name_event(event);
			}
		}
		snapshot
	});
	// Plan owns its range model. Build the separate Day model only while it
	// is shown, or when a block sheet needs its tasks.
	let model = use_memo(move || {
		if page() == Page::Plan && selected_block().is_none() {
			return None;
		}
		snapshot
			.read()
			.as_ref()
			.map(|snapshot| day::build_model(snapshot, &config.read().calendar))
	});
	let travel = use_memo(move || {
		model
			.read()
			.as_ref()
			.map(|model| {
				day::travel_boxes(
					model,
					&config.read().travel,
					config.read().appearance.distance_unit,
				)
			})
			.unwrap_or_default()
	});
	// Filtering affects the grid and its block sheet. Widgets and travel
	// calculations above continue to use the complete day.
	let display = use_memo(move || {
		if search_state.read().is_modified() {
			let settings = config.read().calendar.clone();
			let filtered = model
				.read()
				.as_ref()
				.and(snapshot.read().as_ref())
				.map(|snapshot| {
					day::build_model(
						&day::filter_snapshot(snapshot, &search_state.read(), &settings),
						&settings,
					)
				});
			(filtered, Vec::new())
		} else {
			(model(), travel())
		}
	});
	let calendar_sources = use_memo(move || {
		snapshot()
			.map(|snapshot| snapshot.calendars)
			.unwrap_or_default()
	});
	DayData {
		loaded: data,
		disk_view,
		snapshot,
		#[cfg(target_os = "android")]
		model,
		#[cfg(target_os = "android")]
		travel,
		calendar_sources,
		display,
	}
}

#[component]
pub(super) fn CalendarEffects() -> Element {
	let Navigation {
		current_day,
		page,
		plan_view,
		task_tab,
		mut scrolled,
		mut pending_open,
		..
	} = use_context::<Navigation>();
	let Panels { mut show_notes, .. } = use_context::<Panels>();
	let Selection { mut editing, .. } = use_context::<Selection>();
	let Auth {
		mut connect_error,
		mut signed_in,
		..
	} = use_context::<Auth>();
	let DayData {
		loaded: data,
		disk_view,
		..
	} = use_context::<DayData>();
	let notes_store = use_context::<notes::NotesStore>();
	// Keep the last good day on screen through a transient sync failure.
	use_effect(move || {
		let value = data.read();
		if let Some(Ok(_)) = value.as_ref() {
			let connected = day::signed_in();
			signed_in.set(connected);
			if connected {
				connect_error.set(None);
			}
		}
	});
	// Put the current time in view once, without yanking the scroll position
	// on every later refresh.
	use_effect(move || {
		// Retry when the grid comes into view, if the day loaded on another page.
		let _ = page();
		let _ = plan_view();
		let _ = task_tab();
		let pending = matches!(*data.state().read(), UseResourceState::Pending);
		let loaded_day = match data.read().as_ref() {
			Some(Ok((_, snapshot))) => Some(snapshot.day),
			_ => None,
		};
		let requested_day = current_day();
		if loaded_day == Some(requested_day) && !pending && !scrolled() {
			spawn(async move {
				if matches!(document::eval(SCROLL_SCRIPT).join::<bool>().await, Ok(true))
					&& current_day() == requested_day
				{
					scrolled.set(true);
				}
			});
		}
	});

	// Follow a note's connection or an agenda row once its day is here. Only a
	// finished fetch of that day can say it is gone: a cached copy may simply
	// be older than the event.
	use_effect(move || {
		let Some(id) = pending_open() else {
			return;
		};
		let day = current_day();
		let fetching = matches!(*data.state().read(), UseResourceState::Pending);
		let fetched = match data.read().as_ref() {
			Some(Ok((_, snapshot))) if snapshot.day == day => Some(snapshot.clone()),
			_ => None,
		};
		let known = fetched.clone().or_else(|| {
			disk_view
				.read()
				.as_ref()
				.filter(|(dates, _)| dates == &[day])
				.map(|(_, snapshot)| snapshot.clone())
		});
		// A note connects to a task by its task ID rather than its event's.
		let found = known.and_then(|snapshot| linked_event(snapshot, &id));
		if let Some(event) = found {
			pending_open.set(None);
			editing.set(Some(event));
		} else if fetched.is_some() && !fetching {
			pending_open.set(None);
			let mut message = notes_store.error;
			message.set(Some("That is no longer on its day.".to_owned()));
			show_notes.set(true);
		}
	});

	use_platform_publication();
	rsx! {}
}

fn use_platform_publication() {
	#[cfg(target_os = "android")]
	let DayData {
		loaded: data,
		model: day_model,
		snapshot: snapshot_memo,
		travel: day_travel,
		..
	} = use_context::<DayData>();
	#[cfg(target_os = "android")]
	let Session { config, .. } = use_context::<Session>();
	// Re-plan reminders after a sync or settings change. The sync worker
	// has already persisted source records. Planning reads the database, so it
	// runs on a worker thread.
	#[cfg(target_os = "android")]
	use_effect(move || {
		let _ = data.read();
		reminders::republish(&config.read());
	});
	#[cfg(target_os = "android")]
	use_effect(move || {
		if let (Some(model), Some(snapshot)) = (day_model(), snapshot_memo()) {
			let travel = day_travel();
			let settings = config.read().calendar.clone();
			let today = day::today(&settings);
			spawn(async move {
				let _ = editor::worker(move || {
					widget::publish(&model, &travel, today, &settings, snapshot.fetched_at);
					Ok(())
				})
				.await;
			});
		}
	});
}

/// A retained resource can still hold the previous day while the new one loads.
/// A failed fetch keeps the disk copy available, but only under its own heading.
fn snapshot_for_day(
	day: NaiveDate,
	fetched: Option<&Result<(Vec<NaiveDate>, calendar::Snapshot), String>>,
	cached: impl FnOnce() -> Option<(Vec<NaiveDate>, calendar::Snapshot)>,
) -> Option<calendar::Snapshot> {
	match fetched {
		Some(Ok((_, snapshot))) if snapshot.day == day => Some(snapshot.clone()),
		_ => cached()
			.filter(|(dates, _)| dates == &[day])
			.map(|(_, snapshot)| snapshot),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn navigation_uses_the_requested_days_cache_while_the_resource_retains_another_day() {
		let previous = calendar::sample_snapshot();
		let mut next = previous.clone();
		next.day = previous.day.succ_opt().unwrap();
		next.events.clear();
		let fetched = Ok((vec![previous.day], previous.clone()));
		let cached = (vec![next.day], next.clone());
		assert_eq!(
			snapshot_for_day(next.day, Some(&fetched), || Some(cached.clone())),
			Some(next.clone())
		);
		assert_eq!(snapshot_for_day(next.day, Some(&fetched), || None), None);
		assert_eq!(
			snapshot_for_day(next.day, None, || Some((vec![previous.day], previous))),
			None
		);
	}

	#[test]
	fn a_successful_fetch_replaces_cache_and_a_failure_keeps_it() {
		let cached = calendar::sample_snapshot();
		let day = cached.day;
		let mut fresh = cached.clone();
		fresh.events.clear();
		let cache = (vec![day], cached.clone());
		assert_eq!(
			snapshot_for_day(day, Some(&Ok((vec![day], fresh.clone()))), || panic!(
				"a matching fetch must not subscribe to cache"
			)),
			Some(fresh)
		);
		assert_eq!(
			snapshot_for_day(day, Some(&Err("offline".into())), || Some(cache)),
			Some(cached)
		);
	}
}
