//! Task ordering shared by settings, desktop inbox and mobile planning.
use serde::{Deserialize, Serialize};

use super::Event;
use crate::timeblocking as tb;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxSort {
	#[default]
	Manual,
	CreatedAscending,
	CreatedDescending,
	ModifiedAscending,
	ModifiedDescending,
}

impl InboxSort {
	pub const ALL: [Self; 5] = [
		Self::Manual,
		Self::CreatedAscending,
		Self::CreatedDescending,
		Self::ModifiedAscending,
		Self::ModifiedDescending,
	];

	/// The name it is saved under in the settings.
	pub fn name(self) -> &'static str {
		match self {
			Self::Manual => "manual",
			Self::CreatedAscending => "created_ascending",
			Self::CreatedDescending => "created_descending",
			Self::ModifiedAscending => "modified_ascending",
			Self::ModifiedDescending => "modified_descending",
		}
	}

	pub fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|sort| sort.name() == name)
	}
}

impl std::fmt::Display for InboxSort {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Manual => "Manual order",
			Self::CreatedAscending => "Created: oldest first",
			Self::CreatedDescending => "Created: newest first",
			Self::ModifiedAscending => "Modified: oldest first",
			Self::ModifiedDescending => "Modified: newest first",
		})
	}
}

/// Sorts tasks with unknown timestamps last and deterministic title/id ties.
pub fn sort_tasks(tasks: &mut [&Event], sort: InboxSort) {
	tasks.sort_by_cached_key(|event| {
		let metadata = tb::metadata(event);
		let timestamp = match sort {
			InboxSort::CreatedAscending | InboxSort::CreatedDescending => metadata.created_at,
			InboxSort::ModifiedAscending | InboxSort::ModifiedDescending => {
				event.details.task_updated_at
			}
			InboxSort::Manual => None,
		};
		let descending = matches!(
			sort,
			InboxSort::CreatedDescending | InboxSort::ModifiedDescending
		);
		let manual = if sort == InboxSort::Manual {
			metadata.order
		} else {
			0
		};
		// Keep unknown timestamps last in either direction, and ties stable across syncs.
		(
			timestamp.is_none(),
			timestamp.filter(|_| !descending),
			std::cmp::Reverse(timestamp.filter(|_| descending)),
			manual,
			event.title.to_lowercase(),
			event.id.clone(),
		)
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::{DateTime, Duration};

	fn task(id: &str, title: &str, order: i64) -> Event {
		let mut event = crate::calendar::sample_snapshot().events.remove(0);
		event.id = id.into();
		event.title = title.into();
		event.details = crate::calendar::Details {
			task_data: Some(crate::task_db::TaskData {
				id: 100,
				metadata: tb::Metadata {
					order,
					..Default::default()
				},
			}),
			..Default::default()
		};
		event
	}

	#[test]
	fn both_timestamps_sort_in_both_directions_with_unknowns_last() {
		let early = DateTime::parse_from_rfc3339("2026-10-29T12:00:00Z")
			.unwrap()
			.to_utc();
		let late = early + Duration::seconds(1);
		let mut a = task("a", "A", 2);
		let mut b = task("b", "B", 1);
		let unknown = task("unknown", "Unknown", 0);
		for (event, created, modified) in [(&mut a, early, late), (&mut b, late, early)] {
			event
				.details
				.task_data
				.as_mut()
				.unwrap()
				.metadata
				.created_at = Some(created);
			event.details.task_updated_at = Some(modified);
		}
		for (sort, expected) in [
			(InboxSort::Manual, ["unknown", "b", "a"]),
			(InboxSort::CreatedAscending, ["a", "b", "unknown"]),
			(InboxSort::CreatedDescending, ["b", "a", "unknown"]),
			(InboxSort::ModifiedAscending, ["b", "a", "unknown"]),
			(InboxSort::ModifiedDescending, ["a", "b", "unknown"]),
		] {
			let mut tasks = [&unknown, &b, &a];
			sort_tasks(&mut tasks, sort);
			assert_eq!(tasks.map(|event| event.id.as_str()), expected, "{sort:?}");
		}
	}

	#[test]
	fn ties_use_case_insensitive_titles_then_identity_in_every_mode() {
		let events = [
			task("b", "alpha", 0),
			task("c", "Beta", 0),
			task("a", "ALPHA", 0),
		];
		for sort in InboxSort::ALL {
			let mut tasks: Vec<_> = events.iter().collect();
			for _ in 0..2 {
				sort_tasks(&mut tasks, sort);
				assert_eq!(
					tasks
						.iter()
						.map(|event| event.id.as_str())
						.collect::<Vec<_>>(),
					["a", "b", "c"]
				);
				tasks.reverse();
			}
		}
	}
}
