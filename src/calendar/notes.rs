//! Reading the reserved lines in a Google Task's notes.
//!
//! Google Tasks has no field for a deadline, a clock time, or the app's own
//! metadata, so those live as `KEY: value` lines in the notes. A person can
//! write prose before, after, or between them, so every line is considered:
//!
//! ```text
//! Prepare the report.
//! DEADLINE: 2026-10-30 17:00
//! ```
//!
//! This module is the one place that splits notes into lines. The deadline,
//! metadata, and legacy-id readers all use it, so they agree on where a
//! reserved line may appear.

/// A task's notes, kept in the order they were written.
#[derive(Debug, Clone, Copy)]
pub struct Notes<'a> {
    text: &'a str,
}

impl<'a> Notes<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// The notes exactly as they were stored.
    pub fn text(self) -> &'a str {
        self.text
    }

    /// Every line, including the empty line after a trailing newline.
    pub fn lines(self) -> std::str::Split<'a, char> {
        self.text.split('\n')
    }

    /// The first value written under `key`, wherever its line is.
    pub fn value(self, key: &str) -> Option<&'a str> {
        self.lines().find_map(|line| line_value(line, key))
    }

    /// Every value written under `key`, in order.
    pub fn values<'b>(self, key: &'b str) -> impl Iterator<Item = &'a str> + 'b
    where
        'a: 'b,
    {
        self.lines().filter_map(move |line| line_value(line, key))
    }

    /// The notes without any line that starts with one of `keys`.
    pub fn without(self, keys: &[&str]) -> String {
        self.without_where(|line| keys.iter().any(|key| line_value(line, key).is_some()))
    }

    /// The notes without the lines that `drop` selects.
    pub fn without_where(self, drop: impl Fn(&str) -> bool) -> String {
        self.lines()
            .filter(|line| !drop(line))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The trimmed value after `key` when `line` starts with it.
pub fn line_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.trim_start().strip_prefix(key).map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTES: &str = "First line\nDUE: 2026-10-30\nLast line";

    #[test]
    fn a_key_is_found_on_any_line() {
        assert_eq!(Notes::new(NOTES).value("DUE:"), Some("2026-10-30"));
        assert_eq!(
            Notes::new(NOTES).values("DUE:").collect::<Vec<_>>(),
            ["2026-10-30"]
        );
    }

    #[test]
    fn removing_a_key_leaves_the_other_lines_together() {
        let notes = Notes::new(NOTES);
        assert_eq!(notes.without(&["DUE:"]), "First line\nLast line");
        assert_eq!(notes.without(&["MISSING:"]), NOTES);
        assert_eq!(Notes::new("").without(&["DUE:"]), "");
        assert_eq!(Notes::new("DUE: 2026-10-30\n").without(&["DUE:"]), "");
    }

    #[test]
    fn surrounding_whitespace_does_not_hide_a_key() {
        assert_eq!(
            line_value("  DUE: 2026-10-30  ", "DUE:"),
            Some("2026-10-30")
        );
        assert_eq!(line_value("SOMETHING ELSE", "DUE:"), None);
    }
}
