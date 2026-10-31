//! Display formatting only. Stored values and editor messages remain HH:MM.
use chrono::Timelike;

/// Clock display preference. Legacy boolean settings remain readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeFormat {
    TwelveHour,
    TwentyFourHour,
    Compact,
}

impl TimeFormat {
    pub const ALL: [Self; 3] = [Self::TwelveHour, Self::TwentyFourHour, Self::Compact];

    pub fn is_24_hour(self) -> bool {
        self == Self::TwentyFourHour
    }
}

impl std::fmt::Display for TimeFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TwelveHour => "12-hour time",
            Self::TwentyFourHour => "24-hour time",
            Self::Compact => "Compact Time",
        })
    }
}

impl<'de> serde::Deserialize<'de> for TimeFormat {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Legacy(bool),
            Name(String),
        }
        match Stored::deserialize(deserializer)? {
            Stored::Legacy(true) => Ok(Self::TwentyFourHour),
            Stored::Legacy(false) => Ok(Self::TwelveHour),
            Stored::Name(name) => match name.as_str() {
                "twelve_hour" => Ok(Self::TwelveHour),
                "twenty_four_hour" => Ok(Self::TwentyFourHour),
                "compact" => Ok(Self::Compact),
                _ => Err(serde::de::Error::unknown_variant(
                    &name,
                    &["twelve_hour", "twenty_four_hour", "compact"],
                )),
            },
        }
    }
}

pub fn clock(time: impl Timelike, time_format: TimeFormat) -> String {
    let hour = time.hour();
    let twelve = (hour + 11) % 12 + 1;
    match time_format {
        TimeFormat::TwentyFourHour => format!("{hour:02}:{:02}", time.minute()),
        TimeFormat::TwelveHour => format!(
            "{twelve}:{:02}{}",
            time.minute(),
            if hour < 12 { "AM" } else { "PM" }
        ),
        TimeFormat::Compact => {
            let period = if hour < 12 { "a" } else { "p" };
            if time.minute() == 0 {
                format!("{twelve}{period}")
            } else {
                format!("{twelve}.{:02}{period}", time.minute())
            }
        }
    }
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
    if event.all_day {
        "All day".into()
    } else {
        format!(
            "{} – {}",
            clock(event.start, time_format),
            clock(event.end, time_format)
        )
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
            (0, 7, "12.07a"),
            (1, 0, "1a"),
            (1, 30, "1.30a"),
            (11, 59, "11.59a"),
            (12, 0, "12p"),
            (13, 0, "1p"),
            (13, 5, "1.05p"),
            (23, 47, "11.47p"),
        ] {
            assert_eq!(
                clock(
                    NaiveTime::from_hms_opt(h, m, 0).unwrap(),
                    TimeFormat::Compact
                ),
                expected
            );
        }
        assert_eq!(hour(0, TimeFormat::Compact), "12a");
        assert_eq!(hour(13, TimeFormat::Compact), "1p");
        assert_eq!(hour(24, TimeFormat::Compact), "12a");
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
