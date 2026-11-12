//! Dates and occurrence labels shared by the desktop and phone schedule lists.
use super::{
	Event,
	time_format::{self, TimeFormat},
};
use chrono::{Duration, NaiveDate};

pub const DAYS: i64 = 30;

pub fn dates(anchor: NaiveDate) -> Vec<NaiveDate> {
	(0..DAYS)
		.filter_map(|offset| anchor.checked_add_signed(Duration::days(offset)))
		.collect()
}

/// Calendar dates use an exclusive end, so midnight does not create an extra row.
pub fn occurs_on(event: &Event, day: NaiveDate) -> bool {
	event.start.date_naive() <= day
		&& (day < event.end.date_naive()
			|| (day == event.end.date_naive()
				&& !event.all_day
				&& event.end.time() != chrono::NaiveTime::MIN)
			|| (event.start == event.end && event.start.date_naive() == day))
}

pub fn time_label(event: &Event, format: TimeFormat) -> String {
	if super::deadline::is_marker(event) && !event.all_day {
		time_format::clock(event.start, format)
	} else if event.all_day {
		"All day".into()
	} else {
		time_format::range(event.start, event.end, format)
	}
}

pub fn title(event: &Event, day: NaiveDate) -> String {
	let title = if event.details.sleep.is_some() {
		super::sleep::title(event)
	} else {
		event.title.clone()
	};
	if !event.all_day && event.start.date_naive() != event.end.date_naive() {
		let last = if event.end.time() == chrono::NaiveTime::MIN {
			event.end.date_naive().pred_opt().unwrap_or(day)
		} else {
			event.end.date_naive()
		};
		let total = (last - event.start.date_naive()).num_days() + 1;
		if total > 1 {
			return format!(
				"{} (Day {}/{})",
				title,
				(day - event.start.date_naive()).num_days() + 1,
				total
			);
		}
	}
	title
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn range_starts_on_selected_date_and_crosses_years() {
		let start = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
		let days = dates(start);
		assert_eq!(days.len(), 30);
		assert_eq!(days[0], start);
		assert_eq!(days[1], NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
	}
	#[test]
	fn overnight_rows_have_exclusive_midnight_ends_and_day_labels() {
		use chrono::{Local, TimeZone};
		let mut event = super::super::sample_snapshot().events.remove(0);
		event.title = "Sleep".into();
		event.all_day = false;
		event.start = Local.with_ymd_and_hms(2026, 11, 8, 22, 0, 0).unwrap();
		event.end = Local.with_ymd_and_hms(2026, 11, 9, 6, 0, 0).unwrap();
		let first = event.start.date_naive();
		let second = event.end.date_naive();
		assert!(occurs_on(&event, first));
		assert!(occurs_on(&event, second));
		assert_eq!(title(&event, second), "Sleep (Day 2/2)");
		assert_eq!(
			time_label(&event, TimeFormat::TwentyFourHour),
			"22:00 – 06:00"
		);
		event.end = Local.with_ymd_and_hms(2026, 11, 9, 0, 0, 0).unwrap();
		assert!(!occurs_on(&event, second));
		assert_eq!(title(&event, first), "Sleep");
		event.all_day = true;
		assert!(occurs_on(&event, first));
		assert!(!occurs_on(&event, second));
		assert_eq!(time_label(&event, TimeFormat::TwelveHour), "All day");
	}
}
