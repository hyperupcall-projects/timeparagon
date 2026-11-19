//! Occurrence identity for metadata kept outside Google Calendar events.
//!
//! Google can rename occurrences when a series is retimed or split. Use the
//! same series lineage and original day as time-block membership, rather than
//! treating the new Google ID as a different item.
//!
//! A connection made with "this and following" or "all events" reaches past
//! its one occurrence. It follows the series' lineage, so a series this app
//! or Google split off keeps it, as a time block's tasks do.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{Event, SaveScope};
use crate::timeblocking::membership::{lineage, original_start, parse_instance_id};

/// How much of its series a connection takes in, counted from its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
	/// The one occurrence.
	#[default]
	One,
	/// The occurrence and every later one.
	Following,
	/// Every occurrence.
	All,
}

impl Reach {
	fn is_one(&self) -> bool {
		*self == Self::One
	}
}

impl From<SaveScope> for Reach {
	fn from(scope: SaveScope) -> Self {
		match scope {
			SaveScope::Current => Self::One,
			SaveScope::CurrentAndFuture => Self::Following,
			SaveScope::All => Self::All,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
	pub series: Vec<String>,
	pub day: NaiveDate,
	#[serde(default, skip_serializing_if = "Reach::is_one")]
	pub reach: Reach,
	/// The last day still taken in, once later ones were disconnected.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub until: Option<NaiveDate>,
	/// Days disconnected one at a time.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub except: Vec<NaiveDate>,
}

impl Occurrence {
	pub fn of(event: &Event) -> Option<Self> {
		event.details.recurring_event_id.as_ref()?;
		Some(Self {
			series: lineage(event),
			day: original_start(event).date_naive(),
			reach: Reach::One,
			until: None,
			except: Vec::new(),
		})
	}

	/// This occurrence, reaching as far along its series as `reach` says.
	pub fn reaching(&self, reach: Reach) -> Self {
		Self {
			series: self.series.clone(),
			day: self.day,
			reach,
			until: None,
			except: Vec::new(),
		}
	}

	/// Whether both are the one same occurrence.
	pub fn same(&self, other: &Self) -> bool {
		self.day == other.day && self.related(&other.series)
	}

	/// Whether `series`, a lineage, is this series or one split from or into it.
	fn related(&self, series: &[String]) -> bool {
		self.series.iter().any(|name| series.contains(name))
	}

	/// Whether the day is among those this takes in.
	fn takes_day(&self, day: NaiveDate) -> bool {
		let from = match self.reach {
			Reach::One => return day == self.day,
			Reach::Following => day >= self.day,
			Reach::All => true,
		};
		from && self.until.is_none_or(|until| day <= until) && !self.except.contains(&day)
	}

	/// Whether this connection takes in the one occurrence `other`.
	pub fn covers(&self, other: &Self) -> bool {
		match self.reach {
			Reach::One => self.same(other),
			_ => self.related(&other.series) && self.takes_day(other.day),
		}
	}

	/// Whether this connection takes in everything `other` does.
	pub fn subsumes(&self, other: &Self) -> bool {
		if other.reach == Reach::One {
			return self.covers(other);
		}
		if self.until.is_some() || !self.except.is_empty() || !self.related(&other.series) {
			return false;
		}
		match (self.reach, other.reach) {
			(Reach::All, _) => true,
			(Reach::Following, Reach::Following) => other.day >= self.day,
			_ => false,
		}
	}

	/// Older metadata contains only an event ID and, for notes, a remembered
	/// day. Read those connections too, without rewriting users' notes.
	pub fn matches_id(&self, id: &str, day: Option<NaiveDate>) -> bool {
		let (series, day) = match parse_instance_id(id) {
			Some((series, start)) => (series, Some(start.date_naive())),
			None => (id, day),
		};
		day.is_some_and(|day| self.takes_day(day))
			&& self.series.iter().any(|name| {
				name.rsplit_once('/')
					.is_some_and(|(_, known)| known == series)
			})
	}

	/// What is left of this connection once `cut` is disconnected, or `None`
	/// when nothing is. `cut` is one occurrence reaching as its scope says.
	pub fn without(mut self, cut: &Self) -> Option<Self> {
		if !self.related(&cut.series) {
			return Some(self);
		}
		if self.reach == Reach::One {
			return (!cut.covers(&self)).then_some(self);
		}
		match cut.reach {
			Reach::One => {
				if self.takes_day(cut.day) {
					self.except.push(cut.day);
				}
			}
			Reach::Following => {
				let last = cut.day.pred_opt()?;
				self.until = Some(self.until.map_or(last, |until| until.min(last)));
				if self.reach == Reach::Following && self.day > last {
					return None;
				}
				self.except.retain(|day| *day <= last);
			}
			Reach::All => return None,
		}
		Some(self)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn day(n: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, n).unwrap()
	}

	fn at(series: &[&str], n: u32) -> Occurrence {
		Occurrence {
			series: series.iter().map(|s| (*s).to_owned()).collect(),
			day: day(n),
			reach: Reach::One,
			until: None,
			except: Vec::new(),
		}
	}

	#[test]
	fn a_wider_reach_takes_in_the_series_and_series_split_from_it() {
		let anchor = at(&["app/s"], 10);
		let following = anchor.reaching(Reach::Following);
		let all = anchor.reaching(Reach::All);
		assert!(!anchor.covers(&at(&["app/s"], 11)));
		assert!(following.covers(&at(&["app/s"], 10)));
		assert!(following.covers(&at(&["app/s"], 17)));
		assert!(!following.covers(&at(&["app/s"], 3)));
		assert!(all.covers(&at(&["app/s"], 3)));
		// "This and following" edited later starts a series continuing it.
		assert!(following.covers(&at(&["app/s", "app/t"], 24)));
		assert!(!all.covers(&at(&["app/other"], 10)));
	}

	#[test]
	fn disconnecting_trims_a_wider_connection() {
		let all = at(&["app/s"], 10).reaching(Reach::All);
		let one = all.clone().without(&at(&["app/s"], 17)).unwrap();
		assert!(!one.covers(&at(&["app/s"], 17)));
		assert!(one.covers(&at(&["app/s"], 24)));
		let ended = one
			.without(&at(&["app/s"], 20).reaching(Reach::Following))
			.unwrap();
		assert!(ended.covers(&at(&["app/s"], 3)));
		assert!(!ended.covers(&at(&["app/s"], 20)));
		assert!(!ended.covers(&at(&["app/s"], 24)));
		assert_eq!(ended.except, [day(17)]);
		assert_eq!(
			at(&["app/s"], 12)
				.reaching(Reach::Following)
				.without(&at(&["app/s"], 10).reaching(Reach::Following)),
			None
		);
		assert_eq!(all.without(&at(&["app/s"], 3).reaching(Reach::All)), None);
		let other = at(&["app/other"], 10).reaching(Reach::All);
		assert_eq!(
			other
				.clone()
				.without(&at(&["app/s"], 10).reaching(Reach::All)),
			Some(other)
		);
	}

	#[test]
	fn a_connection_subsumes_only_what_it_wholly_takes_in() {
		let following = at(&["app/s"], 10).reaching(Reach::Following);
		let all = at(&["app/s"], 10).reaching(Reach::All);
		assert!(all.subsumes(&following));
		assert!(following.subsumes(&at(&["app/s"], 12).reaching(Reach::Following)));
		assert!(!following.subsumes(&all));
		assert!(!following.subsumes(&at(&["app/s"], 3)));
		let trimmed = all.clone().without(&at(&["app/s"], 12)).unwrap();
		assert!(!trimmed.subsumes(&following));
	}

	#[test]
	fn connections_saved_before_reach_read_as_one_occurrence() {
		let json = r#"{"series":["app/s"],"day":"2026-11-10"}"#;
		let read: Occurrence = serde_json::from_str(json).unwrap();
		assert_eq!(read, at(&["app/s"], 10));
		assert_eq!(serde_json::to_string(&read).unwrap(), json);
	}
}
