//! How timed events share a column, the way Google Calendar's day view does.
//!
//! Overlapping events are packed into columns left to right. Each cluster of
//! events that touch each other shares the same column count, so a pair of
//! overlapping meetings each take half the width, matching the web UI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_min: i32,
    pub end_min: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packed {
    pub column: usize,
    pub columns: usize,
}

/// Assigns columns to timed events.
///
/// `spans` is parallel to the caller's event list; the returned vec is in the
/// same order. Zero-length or inverted spans still get a slot so a malformed
/// event does not vanish from the grid.
pub fn pack(spans: &[Span]) -> Vec<Packed> {
    let n = spans.len();
    if n == 0 {
        return Vec::new();
    }

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&index| (spans[index].start_min, spans[index].end_min));

    let mut column = vec![0usize; n];
    let mut column_end: Vec<i32> = Vec::new();

    for &index in &order {
        let span = normalized(spans[index]);
        let chosen = column_end
            .iter()
            .position(|&end| end <= span.start_min)
            .unwrap_or_else(|| {
                column_end.push(span.start_min);
                column_end.len() - 1
            });

        column[index] = chosen;
        column_end[chosen] = span.end_min;
    }

    // Connected clusters share a column count, so a later non-overlapping
    // event does not force earlier ones to stay skinny.
    let mut columns = vec![1usize; n];
    let mut cluster = Vec::new();
    let mut cluster_end = i32::MIN;

    for &index in &order {
        let span = normalized(spans[index]);
        if cluster.is_empty() || span.start_min < cluster_end {
            cluster.push(index);
            cluster_end = cluster_end.max(span.end_min);
        } else {
            apply_cluster(&cluster, &column, &mut columns);
            cluster.clear();
            cluster.push(index);
            cluster_end = span.end_min;
        }
    }
    apply_cluster(&cluster, &column, &mut columns);

    (0..n)
        .map(|index| Packed {
            column: column[index],
            columns: columns[index].max(1),
        })
        .collect()
}

fn apply_cluster(cluster: &[usize], column: &[usize], columns: &mut [usize]) {
    let width = cluster
        .iter()
        .map(|&index| column[index] + 1)
        .max()
        .unwrap_or(1);
    for &index in cluster {
        columns[index] = width;
    }
}

fn normalized(span: Span) -> Span {
    if span.end_min <= span.start_min {
        Span {
            start_min: span.start_min,
            end_min: span.start_min + 15,
        }
    } else {
        span
    }
}

/// Minutes from midnight, clamped to a single day so the grid never draws
/// off either end.
pub fn clamp_day(start_min: i32, end_min: i32) -> Option<(i32, i32)> {
    let start = start_min.clamp(0, 24 * 60);
    let end = end_min.clamp(0, 24 * 60);

    if end <= 0 || start >= 24 * 60 || end <= start {
        None
    } else {
        Some((start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_overlapping_events_share_the_full_width() {
        let packed = pack(&[
            Span {
                start_min: 9 * 60,
                end_min: 10 * 60,
            },
            Span {
                start_min: 11 * 60,
                end_min: 12 * 60,
            },
        ]);

        assert_eq!(
            packed,
            vec![
                Packed {
                    column: 0,
                    columns: 1
                },
                Packed {
                    column: 0,
                    columns: 1
                },
            ]
        );
    }

    #[test]
    fn overlapping_events_split_the_row() {
        let packed = pack(&[
            Span {
                start_min: 9 * 60,
                end_min: 11 * 60,
            },
            Span {
                start_min: 10 * 60,
                end_min: 12 * 60,
            },
        ]);

        assert_eq!(packed[0].columns, 2);
        assert_eq!(packed[1].columns, 2);
        assert_ne!(packed[0].column, packed[1].column);
    }

    #[test]
    fn a_cluster_does_not_widen_a_later_unrelated_event() {
        let packed = pack(&[
            Span {
                start_min: 9 * 60,
                end_min: 10 * 60,
            },
            Span {
                start_min: 9 * 60 + 30,
                end_min: 10 * 60 + 30,
            },
            Span {
                start_min: 14 * 60,
                end_min: 15 * 60,
            },
        ]);

        assert_eq!(packed[0].columns, 2);
        assert_eq!(packed[1].columns, 2);
        assert_eq!(packed[2].columns, 1);
        assert_eq!(packed[2].column, 0);
    }

    #[test]
    fn inverted_spans_still_occupy_a_slot() {
        let packed = pack(&[Span {
            start_min: 60,
            end_min: 60,
        }]);

        assert_eq!(packed[0].columns, 1);
    }

    #[test]
    fn events_outside_the_day_are_dropped() {
        assert_eq!(clamp_day(-60, -10), None);
        assert_eq!(clamp_day(24 * 60, 25 * 60), None);
        assert_eq!(clamp_day(-30, 30), Some((0, 30)));
        assert_eq!(clamp_day(23 * 60, 25 * 60), Some((23 * 60, 24 * 60)));
    }
}
