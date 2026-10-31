//! Quick-add syntax. Ranges are UTF-8 byte offsets into the untouched title.
use crate::{config::ProjectsSettings, timeblocking::Priority};
use chrono::{Datelike, Duration, NaiveDate};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Due(NaiveDate),
    Deadline(NaiveDate),
    Label(String),
    Project(u64),
    Priority(Priority),
    Minutes(u32),
}
#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub value: Value,
}

pub fn date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let input = input.trim().to_lowercase();
    let words: Vec<_> = input.split_whitespace().collect();
    let input = words.join(" ");
    match input.as_str() {
        "today" => return Some(today),
        "tomorrow" => return today.checked_add_signed(Duration::days(1)),
        "next week" => return today.checked_add_signed(Duration::days(7)),
        _ => {}
    }
    if let ["in", count, unit] = words.as_slice() {
        let count: i64 = count.parse().ok()?;
        let days = match *unit {
            "day" | "days" => count,
            "week" | "weeks" => count.checked_mul(7)?,
            _ => return None,
        };
        return today.checked_add_signed(Duration::try_days(days)?);
    }
    let weekdays = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    let weekday = input.strip_prefix("next ").unwrap_or(&input);
    if let Some(index) = weekdays
        .iter()
        .position(|name| *name == weekday || name[..3] == *weekday)
    {
        let delta = (index as i64 - today.weekday().num_days_from_monday() as i64).rem_euclid(7);
        return today.checked_add_signed(Duration::days(if delta == 0 { 7 } else { delta }));
    }
    if let Ok(day) = NaiveDate::parse_from_str(&input, "%Y-%m-%d") {
        return Some(day);
    }
    let clean = input.replace(',', "");
    for format in ["%b %d %Y", "%B %d %Y", "%d %b %Y", "%d %B %Y", "%m/%d/%Y"] {
        if let Ok(day) = NaiveDate::parse_from_str(&clean, format) {
            return Some(day);
        }
    }
    for format in ["%b %d %Y", "%B %d %Y", "%d %b %Y", "%d %B %Y", "%m/%d %Y"] {
        if let Ok(mut day) = NaiveDate::parse_from_str(&format!("{clean} {}", today.year()), format)
        {
            if day < today {
                day = day.with_year(today.year() + 1)?;
            }
            return Some(day);
        }
    }
    None
}

pub fn detect(text: &str, today: NaiveDate, projects: &ProjectsSettings) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap();
        if ch.is_whitespace() {
            i += ch.len_utf8();
            continue;
        }
        let start = i;
        let (end, raw) = if ch == '{' {
            if let Some(close) = text[i..].find('}') {
                (i + close + 1, text[i + 1..i + close].trim())
            } else {
                i += ch.len_utf8();
                continue;
            }
        } else {
            let end = text[i..]
                .find(char::is_whitespace)
                .map_or(text.len(), |n| i + n);
            (end, &text[i..end])
        };
        let value = match raw {
            "p1" | "!1" => Some(Value::Priority(Priority::P1)),
            "p2" | "!2" => Some(Value::Priority(Priority::P2)),
            _ if raw.starts_with('@') && raw.len() > 1 => Some(Value::Label(raw[1..].to_owned())),
            _ if raw.starts_with('#') => projects
                .items
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&raw[1..]))
                .map(|p| Value::Project(p.id)),
            _ if raw.starts_with('~') => raw[1..]
                .strip_suffix('m')
                .and_then(|n| n.parse::<u32>().ok())
                .filter(|n| *n > 0 && *n <= 10080)
                .map(Value::Minutes),
            _ => date(raw, today).map(Value::Due),
        };
        // Prefer the longest supported date phrase (e.g. “in 2 weeks”).
        let value = value.map(|value| match value {
            Value::Due(day) if ch == '{' => Value::Deadline(day),
            other => other,
        });
        let mut found = value.map(|value| Token {
            range: start..end,
            value,
        });
        if ch != '{' && !matches!(ch, '@' | '#' | '~' | '!') {
            let mut phrase_end = end;
            for _ in 0..3 {
                let rest = &text[phrase_end..];
                let spaces = rest.len() - rest.trim_start().len();
                if spaces == 0 || spaces == rest.len() {
                    break;
                }
                let next = phrase_end + spaces;
                phrase_end = text[next..]
                    .find(char::is_whitespace)
                    .map_or(text.len(), |n| next + n);
                if let Some(day) = date(&text[start..phrase_end], today) {
                    found = Some(Token {
                        range: start..phrase_end,
                        value: Value::Due(day),
                    });
                }
            }
        }
        if let Some(token) = found {
            i = token.range.end;
            tokens.push(token);
        } else {
            i = end;
        }
    }
    tokens
}

/// Keep dismissed occurrences attached to unchanged text through surrounding edits.
/// Editing the occurrence itself makes it eligible for detection again.
pub fn remap_ignored(old: &str, new: &str, ranges: &mut Vec<Range<usize>>) {
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    ranges.retain_mut(|range| {
        if range.end <= prefix {
            true
        } else if range.start >= old_end {
            *range = (new_end + range.start - old_end)..(new_end + range.end - old_end);
            true
        } else {
            false
        }
    });
}

pub fn remove(text: &str, ranges: impl IntoIterator<Item = Range<usize>>) -> String {
    let mut result = text.to_owned();
    let mut ranges: Vec<_> = ranges.into_iter().collect();
    ranges.sort_by_key(|r| r.start);
    for range in ranges.into_iter().rev() {
        result.replace_range(range, "");
    }
    result.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_dates_projects_and_braced_names_without_eating_unknown_text() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let mut projects = ProjectsSettings::default();
        let project = projects.add("Deep Work".into());
        let text = "Write {#Deep Work} {@deep work} tomorrow {oct 8} !2 ~45m";
        let tokens = detect(text, today, &projects);
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].value, Value::Project(project));
        assert_eq!(tokens[1].value, Value::Label("deep work".into()));
        assert_eq!(tokens[2].value, Value::Due(today.succ_opt().unwrap()));
        assert!(matches!(tokens[3].value, Value::Deadline(_)));
        assert_eq!(tokens[4].value, Value::Priority(Priority::P2));
        assert_eq!(remove(text, tokens.into_iter().map(|t| t.range)), "Write");
        assert!(
            detect(
                "Email me@example.com #unknown {not a date} ~0m !3",
                today,
                &projects
            )
            .is_empty()
        );
        for input in [
            "October 8 2027",
            "8 oct 2027",
            "2027-10-08",
            "10/8/2027",
            "10/8",
        ] {
            assert_eq!(
                date(input, today),
                NaiveDate::from_ymd_opt(2027, 10, 8),
                "{input}"
            );
        }
        assert!(date("feb 30", today).is_none());
    }

    #[test]
    fn dismissed_ranges_follow_unicode_edits_and_drop_when_replaced() {
        let mut ranges = vec![6..14, 15..20];
        remap_ignored("Café tomorrow @home", "🍵 Café tomorrow @home", &mut ranges);
        assert_eq!(ranges, [11..19, 20..25]);
        remap_ignored("🍵 Café tomorrow @home", "🍵 Café today @home", &mut ranges);
        assert_eq!(ranges, [17..22]);
        remap_ignored("🍵 Café today @home", "🍵 Café today @home !1", &mut ranges);
        assert_eq!(ranges, [17..22]);
    }

    #[test]
    fn dates_and_tokens_preserve_prose_and_unicode() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        let text = "  Café {oct 8} @home !1 in 2 weeks ~30m  ";
        let tokens = detect(text, today, &ProjectsSettings::default());
        assert_eq!(tokens.len(), 5);
        assert_eq!(
            tokens[0].value,
            Value::Deadline(NaiveDate::from_ymd_opt(2027, 10, 8).unwrap())
        );
        assert_eq!(remove(text, tokens.into_iter().map(|t| t.range)), "  Café");
        assert!(
            detect(
                "email a@b.com {nonsense} !3",
                today,
                &ProjectsSettings::default()
            )
            .is_empty()
        );
        assert_eq!(
            date("next monday", today),
            NaiveDate::from_ymd_opt(2026, 11, 2)
        );
    }
}
