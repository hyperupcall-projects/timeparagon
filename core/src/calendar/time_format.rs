//! Display formatting only. Stored values and editor messages remain HH:MM.
use chrono::Timelike;

/// Clock display preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormat {
	TwelveHour,
	/// "9:30p"
	Compact,
	TwentyFourHour,
}

impl TimeFormat {
	pub const ALL: [Self; 3] = [Self::TwelveHour, Self::Compact, Self::TwentyFourHour];

	pub fn is_24_hour(self) -> bool {
		self == Self::TwentyFourHour
	}

	/// The compact style uses "a"/"p" periods.
	pub fn is_compact(self) -> bool {
		self == Self::Compact
	}
}

impl std::fmt::Display for TimeFormat {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::TwelveHour => "12-hour (AM/PM)",
			Self::Compact => "12-hour (a/p)",
			Self::TwentyFourHour => "24-hour",
		})
	}
}

/// The time without its AM/PM suffix, plus that suffix ("" for 24-hour).
fn parts(time: &impl Timelike, time_format: TimeFormat) -> (String, &'static str) {
	let hour = time.hour();
	let minute = time.minute();
	let twelve = (hour + 11) % 12 + 1;
	let pm = hour >= 12;
	match time_format {
		TimeFormat::TwentyFourHour => (format!("{hour:02}:{minute:02}"), ""),
		TimeFormat::TwelveHour => (
			format!("{twelve}:{minute:02}"),
			if pm { "PM" } else { "AM" },
		),
		TimeFormat::Compact => {
			let period = if pm { "p" } else { "a" };
			if minute == 0 {
				(twelve.to_string(), period)
			} else {
				(format!("{twelve}:{minute:02}"), period)
			}
		}
	}
}

pub fn clock(time: impl Timelike, time_format: TimeFormat) -> String {
	let (time, period) = parts(&time, time_format);
	time + period
}

/// Both ends of a time range. When both share a period, only the end keeps it,
/// e.g. "9" and "9:30p" rather than "9p" and "9:30p".
pub fn range_parts(
	start: impl Timelike,
	end: impl Timelike,
	time_format: TimeFormat,
) -> (String, String) {
	let (start, start_period) = parts(&start, time_format);
	let (end, end_period) = parts(&end, time_format);
	let start = if start_period == end_period {
		start
	} else {
		start + start_period
	};
	(start, end + end_period)
}

/// A time range such as "9 – 9:30p".
pub fn range(start: impl Timelike, end: impl Timelike, time_format: TimeFormat) -> String {
	let (start, end) = range_parts(start, end, time_format);
	format!("{start} – {end}")
}

pub fn hour(hour: u32, time_format: TimeFormat) -> String {
	match time_format {
		TimeFormat::TwentyFourHour => format!("{:02}:00", hour % 24),
		TimeFormat::TwelveHour => super::hour_label(hour),
		TimeFormat::Compact => clock(
			chrono::NaiveTime::from_hms_opt(hour % 24, 0, 0).unwrap(),
			time_format,
		),
	}
}

pub fn event(event: &super::Event, time_format: TimeFormat) -> String {
	if super::deadline::is_marker(event) {
		return clock(event.start, time_format);
	}
	if event.all_day {
		"All day".into()
	} else {
		range(event.start, event.end, time_format)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::NaiveTime;

	#[test]
	fn compact_time_omits_zero_minutes_and_uses_short_periods() {
		for (h, m, expected) in [
			(0, 0, "12a"),
			(0, 7, "12:07a"),
			(1, 0, "1a"),
			(1, 30, "1:30a"),
			(11, 59, "11:59a"),
			(12, 0, "12p"),
			(13, 0, "1p"),
			(13, 5, "1:05p"),
			(21, 30, "9:30p"),
			(23, 47, "11:47p"),
		] {
			let time = NaiveTime::from_hms_opt(h, m, 0).unwrap();
			assert_eq!(clock(time, TimeFormat::Compact), expected);
		}
		assert_eq!(hour(0, TimeFormat::Compact), "12a");
		assert_eq!(hour(13, TimeFormat::Compact), "1p");
		assert_eq!(hour(24, TimeFormat::Compact), "12a");
	}

	#[test]
	fn ranges_drop_a_shared_period_from_the_start() {
		let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
		for (start, end, format, expected) in [
			(t(21, 0), t(21, 30), TimeFormat::Compact, "9 – 9:30p"),
			(t(21, 0), t(21, 30), TimeFormat::TwelveHour, "9:00 – 9:30PM"),
			(t(11, 30), t(12, 30), TimeFormat::Compact, "11:30a – 12:30p"),
			(
				t(11, 30),
				t(12, 30),
				TimeFormat::TwelveHour,
				"11:30AM – 12:30PM",
			),
			(t(23, 0), t(1, 0), TimeFormat::Compact, "11p – 1a"),
			(
				t(21, 0),
				t(21, 30),
				TimeFormat::TwentyFourHour,
				"21:00 – 21:30",
			),
		] {
			assert_eq!(range(start, end, format), expected);
		}
	}

	#[test]
	fn time_format_handles_midnight_noon_and_minutes() {
		for (h, m, twelve, twenty_four) in [
			(0, 0, "12:00AM", "00:00"),
			(0, 7, "12:07AM", "00:07"),
			(11, 59, "11:59AM", "11:59"),
			(12, 0, "12:00PM", "12:00"),
			(13, 0, "1:00PM", "13:00"),
			(23, 47, "11:47PM", "23:47"),
		] {
			let time = NaiveTime::from_hms_opt(h, m, 0).unwrap();
			assert_eq!(clock(time, TimeFormat::TwelveHour), twelve);
			assert_eq!(clock(time, TimeFormat::TwentyFourHour), twenty_four);
		}
		assert_eq!(hour(0, TimeFormat::TwelveHour), "12 AM");
		assert_eq!(hour(13, TimeFormat::TwentyFourHour), "13:00");
	}
}
