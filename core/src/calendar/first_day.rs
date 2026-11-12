//! Which weekday a calendar week starts on.
use chrono::{Datelike, NaiveDate, Weekday};

/// The first column of the month and two-week grids and of the date pickers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstDayOfWeek {
	#[default]
	Sunday,
	Monday,
	Saturday,
}

impl FirstDayOfWeek {
	pub const ALL: [Self; 3] = [Self::Saturday, Self::Sunday, Self::Monday];

	/// The name it is stored under in the settings.
	pub fn name(self) -> &'static str {
		match self {
			Self::Sunday => "sunday",
			Self::Monday => "monday",
			Self::Saturday => "saturday",
		}
	}

	pub fn parse(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|first| first.name() == name)
	}

	fn weekday(self) -> Weekday {
		match self {
			Self::Sunday => Weekday::Sun,
			Self::Monday => Weekday::Mon,
			Self::Saturday => Weekday::Sat,
		}
	}

	/// The seven weekdays in the order a week shows them.
	pub fn weekdays(self) -> [Weekday; 7] {
		let mut day = self.weekday();
		[(); 7].map(|()| {
			let current = day;
			day = day.succ();
			current
		})
	}

	/// Weekday names for column headings and choices, e.g. "Sun".
	pub fn short_labels(self) -> [&'static str; 7] {
		self.weekdays().map(|day| match day {
			Weekday::Mon => "Mon",
			Weekday::Tue => "Tue",
			Weekday::Wed => "Wed",
			Weekday::Thu => "Thu",
			Weekday::Fri => "Fri",
			Weekday::Sat => "Sat",
			Weekday::Sun => "Sun",
		})
	}

	/// Single-letter headings for the compact month pickers.
	pub fn initials(self) -> [&'static str; 7] {
		self.short_labels().map(|label| &label[..1])
	}

	/// The first day of the week holding `day`.
	pub fn week_start(self, day: NaiveDate) -> NaiveDate {
		day - chrono::Duration::days(self.days_into_week(day.weekday()))
	}

	/// How many days after the start of the week `day` falls.
	pub(super) fn days_into_week(self, day: Weekday) -> i64 {
		i64::from(day.days_since(self.weekday()))
	}
}

impl std::fmt::Display for FirstDayOfWeek {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Sunday => "Sunday",
			Self::Monday => "Monday",
			Self::Saturday => "Saturday",
		})
	}
}
