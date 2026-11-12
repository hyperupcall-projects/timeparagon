//! What makes goals and objectives trackable: their status, how an
//! objective's progress is measured, check-ins, milestones, life-area
//! satisfaction ratings and weekly reviews, and the arithmetic that turns them
//! into progress and pace.
//!
//! A **goal** is aspirational: something wanted that could be reached in more
//! than one way, so it has no measure or deadline of its own. Its progress is
//! that of its objectives. An **objective** is concrete (SMART, or a key
//! result): it says how a goal will be reached, has a time frame, and is
//! measured by a number, a checklist of milestones, or the habits that carry
//! it out.
//!
//! Every list item has an `id` or a `date`, which is how the shared document
//! merge tells items apart.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Where a goal or objective stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
	#[default]
	Active,
	/// Set aside for now, without giving up on it.
	Paused,
	/// Reached.
	Done,
	/// Given up on, kept for the record.
	Dropped,
}

impl Status {
	pub const ALL: [Self; 4] = [Self::Active, Self::Paused, Self::Done, Self::Dropped];

	pub fn label(self) -> &'static str {
		match self {
			Self::Active => "Active",
			Self::Paused => "Paused",
			Self::Done => "Done",
			Self::Dropped => "Dropped",
		}
	}

	pub fn is_default(&self) -> bool {
		*self == Self::Active
	}

	/// Still being worked on: neither finished nor set aside.
	pub fn is_open(self) -> bool {
		self == Self::Active
	}
}

/// How an objective's progress is worked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Measure {
	/// Only done or not done.
	#[default]
	None,
	/// From a starting value towards a target, by the latest check-in, such as
	/// a weight of 90 kg down to 80 kg, or 0 to 30 km a week.
	Number,
	/// The share of its milestones that are done.
	Milestones,
	/// The average strength of the habits that carry it out.
	Habits,
}

impl Measure {
	pub const ALL: [Self; 4] = [Self::None, Self::Number, Self::Milestones, Self::Habits];

	pub fn label(self) -> &'static str {
		match self {
			Self::None => "Done / not done",
			Self::Number => "Number",
			Self::Milestones => "Milestones",
			Self::Habits => "Habits",
		}
	}

	pub fn is_default(&self) -> bool {
		*self == Self::None
	}
}

/// A step on the way to an objective.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Milestone {
	pub id: u64,
	pub name: String,
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub done: bool,
}

/// A value recorded for a [`Measure::Number`] objective. One per day: a
/// second check-in on the same day replaces the first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckIn {
	pub date: NaiveDate,
	pub value: f64,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub note: String,
}

/// How satisfied one was with a life area, from 1 to 10, as of a week (the
/// date is the first day of that week).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
	pub date: NaiveDate,
	pub score: u8,
}

/// A weekly review, by the first day of the week it looks back on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
	pub date: NaiveDate,
	/// Markdown: what went well, what did not, what to change.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub reflection: String,
	/// Set once the review has been finished.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub done: bool,
}

/// Whether an open objective is keeping up with its time frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
	Done,
	Paused,
	Dropped,
	/// Its due date has passed.
	Overdue,
	/// Less done than the time gone by would call for.
	Behind,
	OnTrack,
	/// Nothing to compare against: no time frame, or no measured progress.
	Open,
}

impl Pace {
	pub fn label(self) -> &'static str {
		match self {
			Self::Done => "Done",
			Self::Paused => "Paused",
			Self::Dropped => "Dropped",
			Self::Overdue => "Overdue",
			Self::Behind => "Behind",
			Self::OnTrack => "On track",
			Self::Open => "In progress",
		}
	}

	/// Calls for attention.
	pub fn is_alarming(self) -> bool {
		matches!(self, Self::Overdue | Self::Behind)
	}
}

/// How far behind the time gone by progress may fall before it counts as
/// behind.
const PACE_SLACK: f64 = 0.1;

/// Replaces the item with the same date, or adds one, keeping them in date
/// order.
pub(crate) fn upsert_by_date<T>(
	items: &mut Vec<T>,
	date: NaiveDate,
	item: T,
	key: fn(&T) -> NaiveDate,
) {
	match items.binary_search_by_key(&date, key) {
		Ok(index) => items[index] = item,
		Err(index) => items.insert(index, item),
	}
}

/// Progress of a [`Measure::Number`] objective, from its start towards its
/// target. Works the same whether the target is above or below the start.
pub fn number_progress(baseline: f64, target: f64, current: f64) -> Option<f64> {
	let span = target - baseline;
	if !span.is_finite() || span.abs() < f64::EPSILON || !current.is_finite() {
		return None;
	}
	Some(((current - baseline) / span).clamp(0.0, 1.0))
}

/// The pace an objective with `status`, its time frame and `progress` is
/// going at, on `today`.
pub fn pace(
	status: Status,
	start: Option<NaiveDate>,
	due: Option<NaiveDate>,
	progress: Option<f64>,
	today: NaiveDate,
) -> Pace {
	match status {
		Status::Done => return Pace::Done,
		Status::Paused => return Pace::Paused,
		Status::Dropped => return Pace::Dropped,
		Status::Active => {}
	}
	let Some(due) = due else {
		return Pace::Open;
	};
	if due < today {
		return Pace::Overdue;
	}
	let (Some(start), Some(progress)) = (start, progress) else {
		return Pace::Open;
	};
	let length = (due - start).num_days();
	if length <= 0 || today <= start {
		return Pace::OnTrack;
	}
	let elapsed = ((today - start).num_days() as f64 / length as f64).clamp(0.0, 1.0);
	if progress + PACE_SLACK < elapsed {
		Pace::Behind
	} else {
		Pace::OnTrack
	}
}

/// The mean of the values that are there, or none when there are none.
pub fn mean(values: impl IntoIterator<Item = f64>) -> Option<f64> {
	let (sum, count) = values
		.into_iter()
		.fold((0.0, 0usize), |(sum, count), v| (sum + v, count + 1));
	(count > 0).then(|| sum / count as f64)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn day(text: &str) -> NaiveDate {
		NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
	}

	#[test]
	fn number_progress_goes_either_way_and_stays_in_bounds() {
		assert_eq!(number_progress(0.0, 30.0, 15.0), Some(0.5));
		assert_eq!(number_progress(90.0, 80.0, 85.0), Some(0.5));
		assert_eq!(number_progress(90.0, 80.0, 95.0), Some(0.0));
		assert_eq!(number_progress(0.0, 10.0, 12.0), Some(1.0));
		assert_eq!(number_progress(5.0, 5.0, 5.0), None);
	}

	#[test]
	fn pace_compares_progress_with_the_time_gone_by() {
		let start = Some(day("2026-01-01"));
		let due = Some(day("2026-01-11"));
		let today = day("2026-01-06");
		assert_eq!(
			pace(Status::Active, start, due, Some(0.5), today),
			Pace::OnTrack
		);
		assert_eq!(
			pace(Status::Active, start, due, Some(0.2), today),
			Pace::Behind
		);
		assert_eq!(pace(Status::Active, start, due, None, today), Pace::Open);
		assert_eq!(
			pace(Status::Active, None, None, Some(0.0), today),
			Pace::Open
		);
		assert_eq!(
			pace(Status::Active, start, due, Some(0.9), day("2026-01-12")),
			Pace::Overdue
		);
		assert_eq!(
			pace(Status::Done, start, due, Some(0.0), day("2026-01-12")),
			Pace::Done
		);
	}

	#[test]
	fn upserting_by_date_replaces_the_same_day_and_keeps_order() {
		let mut items = vec![(day("2026-01-03"), 1)];
		upsert_by_date(&mut items, day("2026-01-01"), (day("2026-01-01"), 2), |i| {
			i.0
		});
		upsert_by_date(&mut items, day("2026-01-03"), (day("2026-01-03"), 3), |i| {
			i.0
		});
		assert_eq!(items, [(day("2026-01-01"), 2), (day("2026-01-03"), 3)]);
	}
}
