//! Deadlines travel in Google Tasks notes, independently of scheduled dates.
use chrono::NaiveDate;

pub const PREFIX: &str = "DUE: ";

pub fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "Enter a deadline as YYYY-MM-DD".to_owned())?;
    if date.to_string() != value {
        return Err("Enter a deadline as YYYY-MM-DD".into());
    }
    Ok(date)
}

pub fn read(notes: &str) -> Result<Option<NaiveDate>, String> {
    match notes
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("DUE:"))
    {
        Some(value) => parse_date(value.trim()).map(Some),
        None => Ok(None),
    }
}

/// Replace only the reserved first line; preserve the rest of the prose exactly.
pub fn write(notes: &str, date: Option<NaiveDate>) -> String {
    let body = if notes
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("DUE:"))
    {
        notes.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        notes
    };
    match date {
        Some(date) if body.is_empty() => format!("{PREFIX}{date}"),
        Some(date) => format!("{PREFIX}{date}\n{body}"),
        None => body.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_round_trip_without_changing_prose() {
        let date = parse_date("2026-10-30").unwrap();
        let body = "\nPrepare report.\nDUE: this is ordinary prose\n";
        let notes = write(body, Some(date));
        assert_eq!(read(&notes).unwrap(), Some(date));
        assert_eq!(write(&notes, None), body);
        assert_eq!(write(&notes, Some(date)), notes);
        assert_eq!(read(body).unwrap(), None);
    }

    #[test]
    fn malformed_deadlines_can_be_repaired_or_cleared() {
        for value in ["2026-02-30", "2026-1-2", "tomorrow", ""] {
            assert!(parse_date(value).is_err());
            let notes = format!("DUE: {value}\nKeep my notes");
            assert!(read(&notes).is_err());
            assert_eq!(write(&notes, None), "Keep my notes");
        }
    }
}
