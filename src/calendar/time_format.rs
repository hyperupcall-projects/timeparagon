//! Display formatting only. Stored values and editor messages remain HH:MM.
use chrono::Timelike;

pub fn clock(time: impl Timelike, use_24_hour: bool) -> String {
    let hour = time.hour();
    if use_24_hour {
        format!("{hour:02}:{:02}", time.minute())
    } else {
        format!(
            "{}:{:02}{}",
            (hour + 11) % 12 + 1,
            time.minute(),
            if hour < 12 { "AM" } else { "PM" }
        )
    }
}

pub fn hour(hour: u32, use_24_hour: bool) -> String {
    if use_24_hour {
        format!("{:02}:00", hour % 24)
    } else {
        super::hour_label(hour)
    }
}

pub fn event(event: &super::Event, use_24_hour: bool) -> String {
    if event.all_day {
        "All day".into()
    } else {
        format!(
            "{} – {}",
            clock(event.start, use_24_hour),
            clock(event.end, use_24_hour)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

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
            assert_eq!(clock(time, false), twelve);
            assert_eq!(clock(time, true), twenty_four);
        }
        assert_eq!(hour(0, false), "12 AM");
        assert_eq!(hour(13, true), "13:00");
    }
}
