//! What the routines wrote down: the plan and the reflection for each day,
//! week and month, and the daily refreshes.
//!
//! One [`Entry`] per period, found by its [`Period::id`]. Everything is kept
//! in the shared `shared.routines` document, so it syncs between devices like
//! the brain dump. List items carry an `id` (or `recorded_at`) so that
//! concurrent edits merge item by item.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::calendar::first_day::FirstDayOfWeek;

/// How long a period is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scale {
	Day,
	Week,
	Month,
}

impl Scale {
	pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Month];

	/// "day", "week" or "month", as written in prose and in ids.
	pub fn noun(self) -> &'static str {
		match self {
			Self::Day => "day",
			Self::Week => "week",
			Self::Month => "month",
		}
	}
}

/// One day, one week (starting on the configured first day) or one month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Period {
	pub scale: Scale,
	/// The first day in the period.
	pub first: NaiveDate,
}

impl Period {
	/// The period of `scale` that `day` falls in.
	pub fn of(scale: Scale, day: NaiveDate, first_day: FirstDayOfWeek) -> Self {
		let first = match scale {
			Scale::Day => day,
			Scale::Week => {
				let start = first_day.weekdays()[0];
				let back =
					(7 + day.weekday().num_days_from_monday() - start.num_days_from_monday()) % 7;
				day - Duration::days(i64::from(back))
			}
			Scale::Month => day.with_day(1).unwrap_or(day),
		};
		Self { scale, first }
	}

	pub fn day(day: NaiveDate) -> Self {
		Self {
			scale: Scale::Day,
			first: day,
		}
	}

	/// The last day in the period.
	pub fn last(self) -> NaiveDate {
		match self.scale {
			Scale::Day => self.first,
			Scale::Week => self.first + Duration::days(6),
			Scale::Month => self
				.first
				.checked_add_months(Months::new(1))
				.and_then(|next| next.pred_opt())
				.unwrap_or(self.first),
		}
	}

	pub fn contains(self, day: NaiveDate) -> bool {
		self.first <= day && day <= self.last()
	}

	/// Every day in the period, in order.
	pub fn days(self) -> Vec<NaiveDate> {
		self.first
			.iter_days()
			.take_while(|day| *day <= self.last())
			.collect()
	}

	pub fn next(self) -> Self {
		let first = match self.scale {
			Scale::Day => self.first + Duration::days(1),
			Scale::Week => self.first + Duration::days(7),
			Scale::Month => self
				.first
				.checked_add_months(Months::new(1))
				.unwrap_or(self.first),
		};
		Self { first, ..self }
	}

	pub fn previous(self) -> Self {
		let first = match self.scale {
			Scale::Day => self.first - Duration::days(1),
			Scale::Week => self.first - Duration::days(7),
			Scale::Month => self
				.first
				.checked_sub_months(Months::new(1))
				.unwrap_or(self.first),
		};
		Self { first, ..self }
	}

	/// The key its entry is filed under: `day:2026-11-12`, `week:2026-11-09`
	/// (the week's first day) or `month:2026-11`.
	pub fn id(self) -> String {
		match self.scale {
			Scale::Day | Scale::Week => {
				format!("{}:{}", self.scale.noun(), self.first.format("%Y-%m-%d"))
			}
			Scale::Month => format!("month:{}", self.first.format("%Y-%m")),
		}
	}

	/// "Thursday, Nov 12", "Nov 9 – 15", "Oct 26 – Nov 1" or "November 2026".
	pub fn label(self) -> String {
		match self.scale {
			Scale::Day => self.first.format("%A, %b %-d").to_string(),
			Scale::Week => {
				let last = self.last();
				if last.month() == self.first.month() {
					format!("{} – {}", self.first.format("%b %-d"), last.day())
				} else {
					format!(
						"{} – {}",
						self.first.format("%b %-d"),
						last.format("%b %-d")
					)
				}
			}
			Scale::Month => self.first.format("%B %Y").to_string(),
		}
	}
}

/// Something to get done within a week or a month.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
	pub id: u64,
	pub text: String,
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub done: bool,
}

/// A task picked out as one of the period's highlights. The title is kept so
/// the highlight still reads after the task is gone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
	/// The task's key (`timeblocking::task_key`), or a random id for a
	/// highlight that was typed in rather than picked.
	pub id: String,
	pub title: String,
}

/// One pass of the daily refresh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refresh {
	pub recorded_at: DateTime<Utc>,
	/// Tasks a decision was made about.
	pub decided: u32,
	/// Tasks left in Unfinished, Inbox and Today when it finished.
	pub remaining: u32,
}

/// What was planned and reflected for one period.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
	/// [`Period::id`].
	pub id: String,
	/// The plan's one-line focus. For a day this is its highlight: the one
	/// thing that would make it a good day.
	#[serde(skip_serializing_if = "String::is_empty")]
	pub intention: String,
	/// Whether the intention was met, answered in the reflection.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub intention_met: Option<bool>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub objectives: Vec<Objective>,
	/// When the plan was finished.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub planned_at: Option<DateTime<Utc>>,
	/// The tasks committed to when the plan was finished, by task key.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub planned_tasks: Vec<String>,
	/// Their estimates added up, in minutes.
	#[serde(skip_serializing_if = "is_zero")]
	pub planned_minutes: u32,
	/// Reflection answers by prompt id.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub answers: BTreeMap<String, String>,
	/// How the period went, from 1 (rough) to 5 (great).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub rating: Option<u8>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub highlights: Vec<Highlight>,
	/// When the reflection was finished.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub reflected_at: Option<DateTime<Utc>>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub refreshes: Vec<Refresh>,
}

fn is_zero(value: &u32) -> bool {
	*value == 0
}

impl Entry {
	/// Whether nothing but the id is left, so it need not be stored.
	pub fn is_blank(&self) -> bool {
		*self
			== Self {
				id: self.id.clone(),
				..Self::default()
			}
	}

	pub fn answer(&self, prompt: &str) -> &str {
		self.answers.get(prompt).map_or("", String::as_str)
	}

	pub fn is_highlight(&self, id: &str) -> bool {
		self.highlights.iter().any(|highlight| highlight.id == id)
	}
}

/// Every entry, in no particular order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routines {
	pub entries: Vec<Entry>,
}

impl Routines {
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	pub fn get(&self, period: Period) -> Option<&Entry> {
		let id = period.id();
		self.entries.iter().find(|entry| entry.id == id)
	}

	/// The period's entry, created when it is first written to.
	pub fn entry_mut(&mut self, period: Period) -> &mut Entry {
		let id = period.id();
		if let Some(index) = self.entries.iter().position(|entry| entry.id == id) {
			return &mut self.entries[index];
		}
		self.entries.push(Entry {
			id,
			..Entry::default()
		});
		self.entries.last_mut().expect("just pushed")
	}

	/// Drops entries with nothing left in them, so clearing a field does not
	/// leave an empty record behind.
	pub fn tidy(&mut self) {
		self.entries.retain(|entry| !entry.is_blank());
	}
}

/// A fresh id for an objective, random so two devices adding one to the same
/// week at once do not collide.
pub fn new_id() -> u64 {
	let mut bytes = [0; 8];
	getrandom::fill(&mut bytes).expect("objective identity needs system randomness");
	// Kept within what JSON numbers hold exactly, as other ids are.
	u64::from_ne_bytes(bytes) >> 12
}

#[cfg(test)]
mod tests {
	use super::*;

	fn date(y: i32, m: u32, d: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(y, m, d).unwrap()
	}

	#[test]
	fn weeks_start_on_the_configured_day() {
		// Thursday, November 12, 2026.
		let thursday = date(2026, 11, 12);
		let monday = Period::of(Scale::Week, thursday, FirstDayOfWeek::Monday);
		assert_eq!(monday.first, date(2026, 11, 9));
		assert_eq!(monday.last(), date(2026, 11, 15));
		let sunday = Period::of(Scale::Week, thursday, FirstDayOfWeek::Sunday);
		assert_eq!(sunday.first, date(2026, 11, 8));
		let saturday = Period::of(Scale::Week, thursday, FirstDayOfWeek::Saturday);
		assert_eq!(saturday.first, date(2026, 11, 7));
		// The first day of the week is its own week's start.
		let start = Period::of(Scale::Week, date(2026, 11, 9), FirstDayOfWeek::Monday);
		assert_eq!(start.first, date(2026, 11, 9));
		assert_eq!(monday.days().len(), 7);
	}

	#[test]
	fn months_cover_every_day_and_step_cleanly() {
		let november = Period::of(Scale::Month, date(2026, 11, 12), FirstDayOfWeek::Monday);
		assert_eq!(november.first, date(2026, 11, 1));
		assert_eq!(november.last(), date(2026, 11, 30));
		assert_eq!(november.days().len(), 30);
		assert_eq!(november.next().first, date(2026, 12, 1));
		assert_eq!(november.previous().first, date(2026, 10, 1));
		let february = Period::of(Scale::Month, date(2028, 2, 3), FirstDayOfWeek::Monday);
		assert_eq!(february.last(), date(2028, 2, 29));
	}

	#[test]
	fn ids_and_labels_name_the_period() {
		let day = Period::day(date(2026, 11, 12));
		assert_eq!(day.id(), "day:2026-11-12");
		assert_eq!(day.label(), "Thursday, Nov 12");
		let week = Period::of(Scale::Week, date(2026, 11, 12), FirstDayOfWeek::Monday);
		assert_eq!(week.id(), "week:2026-11-09");
		assert_eq!(week.label(), "Nov 9 – 15");
		let across = Period::of(Scale::Week, date(2026, 10, 29), FirstDayOfWeek::Monday);
		assert_eq!(across.label(), "Oct 26 – Nov 1");
		let month = Period::of(Scale::Month, date(2026, 11, 12), FirstDayOfWeek::Monday);
		assert_eq!(month.id(), "month:2026-11");
		assert_eq!(month.label(), "November 2026");
	}

	#[test]
	fn entries_are_made_on_first_write_and_tidied_away_when_blank() {
		let mut routines = Routines::default();
		let day = Period::day(date(2026, 11, 12));
		assert!(routines.get(day).is_none());
		routines.entry_mut(day).intention = "Ship it".into();
		assert_eq!(routines.get(day).unwrap().intention, "Ship it");
		routines.entry_mut(day).rating = Some(4);
		assert_eq!(routines.entries.len(), 1);
		routines.entry_mut(day).intention.clear();
		routines.entry_mut(day).rating = None;
		routines.tidy();
		assert!(routines.is_empty());
	}

	#[test]
	fn the_document_leaves_out_what_is_unset() {
		let mut routines = Routines::default();
		let day = Period::day(date(2026, 11, 12));
		routines.entry_mut(day).intention = "Ship it".into();
		let json = serde_json::to_string(&routines).unwrap();
		assert_eq!(
			json,
			r#"{"entries":[{"id":"day:2026-11-12","intention":"Ship it"}]}"#
		);
		let back: Routines = serde_json::from_str(&json).unwrap();
		assert_eq!(back, routines);
	}
}
