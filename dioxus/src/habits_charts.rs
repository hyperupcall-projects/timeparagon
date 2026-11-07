//! The marks and charts of the habit screens, drawn as the desktop draws them
//! (`src/ui/habits_charts.rs`), which is the way Loop Habit Tracker does.
//!
//! The desktop pans a chart with a drag. Here each chart is as wide as its
//! history and scrolls sideways, starting at the newest, so a swipe pages
//! through the past as it does in Loop. Colors are the `--habit-*` variables
//! from `style.rs`, so they follow the theme like the desktop's do.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use dioxus::prelude::*;
use timeslay_core::calendar::month_short;
use timeslay_core::habits::{self, Bucket, Computed, Icons, Kind, Status, Streak};

/// The week the charts group by, matching the desktop.
pub const FIRST_WEEKDAY: Weekday = Weekday::Mon;
pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const TEXT_SIZE: f32 = 11.0;
const AXIS_HEIGHT: f32 = 32.0;
const COLUMN_WIDTH: f32 = 34.0;
/// Columns drawn however short the history, so a new habit's chart still
/// spans a phone's width.
const MIN_COLUMNS: usize = 12;

/// The inline style giving an element a habit's color and the ink on it.
pub fn habit_color(index: u8) -> String {
    let index = index % habits::PALETTE_SIZE;
    format!("--hc: var(--habit-{index}); --hc-ink: var(--habit-ink-{index});")
}

/// `color` blended over `under`, `share` of the way.
fn mix(color: &str, share: f32, under: &str) -> String {
    format!(
        "color-mix(in srgb, {color} {:.0}%, {under})",
        share.clamp(0.0, 1.0) * 100.0
    )
}

const HC: &str = "var(--hc)";
const LOW: &str = "var(--habit-low)";
const MEDIUM: &str = "var(--habit-medium)";
const QUIET: &str = "var(--habit-quiet)";
const CARD: &str = "var(--card)";
/// What a slip is drawn in: the palette's red.
const SLIP: &str = "var(--habit-0)";

fn fmt(value: f32) -> String {
    format!("{value:.2}")
}

// --------------------------------------------------------------------- ring

/// Loop's score ring: a track with the score swept clockwise from the top.
#[component]
pub fn Ring(score: f64, size: f32) -> Element {
    let score = score.clamp(0.0, 1.0) as f32;
    let thickness = (size * 0.22).max(2.5);
    let radius = size / 2.0 - thickness / 2.0;
    let circumference = std::f32::consts::TAU * radius;
    let swept = circumference * score;
    let center = fmt(size / 2.0);
    rsx! {
        svg {
            class: "habit-ring",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 {size} {size}",
            "aria-hidden": "true",
            circle {
                cx: "{center}",
                cy: "{center}",
                r: "{fmt(radius)}",
                style: "fill: none; stroke: {LOW}; stroke-width: {fmt(thickness)};",
            }
            if score > 0.0 {
                circle {
                    cx: "{center}",
                    cy: "{center}",
                    r: "{fmt(radius)}",
                    transform: "rotate(-90 {center} {center})",
                    style: "fill: none; stroke: {HC}; stroke-width: {fmt(thickness)}; stroke-dasharray: {fmt(swept)} {fmt(circumference)};",
                }
            }
        }
    }
}

// ---------------------------------------------------------------- day marks

/// What a day's mark shows, as on the desktop's list.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub status: Status,
    pub goal: u32,
    pub question_marks: bool,
    pub icons: Icons,
    pub kind: Kind,
    /// How many times it was done, shown when more than once.
    pub times: u32,
    /// On a "No" habit: how many times it was resisted, shown on a clean day.
    pub resisted: u32,
    /// The number recorded for the day.
    pub value: Option<f64>,
}

impl Mark {
    pub fn of(habit: &habits::Habit, status: Status, day: NaiveDate) -> Self {
        Self {
            status,
            goal: habit.frequency.per_day(),
            question_marks: habit.question_marks,
            icons: habit.icons,
            kind: habit.kind,
            times: habit.times_on(day),
            resisted: habit.resisted_on(day),
            value: habit.measure.as_ref().and(habit.value_on(day)),
        }
    }

    /// A sample of one status, for the form's icon choice.
    pub fn sample(status: Status, icons: Icons) -> Self {
        Self {
            status,
            goal: 1,
            question_marks: false,
            icons,
            kind: Kind::Yes,
            times: 0,
            resisted: 0,
            value: None,
        }
    }

    /// The face a day shows, and its color. A partial count says something a
    /// face cannot, so it keeps what it always shows.
    fn face(&self) -> Option<(Face, &'static str, &'static str)> {
        const QUIET_INK: &str = "var(--habit-quiet-ink)";
        const SLIP_INK: &str = "var(--habit-ink-0)";
        const INK: &str = "var(--hc-ink)";
        match self.status {
            Status::Yes | Status::Clean => Some((Face::Happy, HC, INK)),
            Status::Auto => Some((Face::Happy, QUIET, QUIET_INK)),
            Status::Slipped(_) => Some((Face::Disappointed, SLIP, SLIP_INK)),
            Status::Skip => Some((Face::Question, HC, INK)),
            Status::No => Some((Face::Disappointed, QUIET, QUIET_INK)),
            Status::Unknown if self.question_marks => Some((Face::Question, QUIET, QUIET_INK)),
            Status::Unknown if self.kind == Kind::No => None,
            Status::Unknown => Some((Face::Disappointed, QUIET, QUIET_INK)),
            Status::Partial(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Happy,
    Disappointed,
    Question,
}

const CHECK: &[(f32, f32)] = &[(-5.5, 0.5), (-1.5, 4.5), (5.5, -4.0)];

fn polyline(points: &[(f32, f32)], color: &str, width: f32) -> Element {
    let points: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("{},{}", fmt(*x), fmt(*y)))
        .collect();
    let points = points.join(" ");
    rsx! {
        polyline {
            points: "{points}",
            style: "fill: none; stroke: {color}; stroke-width: {width}; stroke-linecap: round; stroke-linejoin: round;",
        }
    }
}

fn small_text(content: String, x: f32, y: f32, color: &str, size: f32) -> Element {
    rsx! {
        text {
            x: "{fmt(x)}",
            y: "{fmt(y)}",
            "text-anchor": "middle",
            "dominant-baseline": "central",
            style: "fill: {color}; font-size: {size}px;",
            "{content}"
        }
    }
}

/// The mark in a day's button: a glyph centred in a 40 by 44 box, with the
/// day's count and value around it.
#[component]
pub fn DayMark(mark: Mark, width: Option<f32>, height: Option<f32>) -> Element {
    let width = width.unwrap_or(40.0);
    let height = height.unwrap_or(44.0);
    let (cx, cy) = (width / 2.0, height / 2.0);
    let at = |points: &[(f32, f32)]| -> Vec<(f32, f32)> {
        points.iter().map(|(x, y)| (cx + x, cy + y)).collect()
    };
    let mut parts: Vec<Element> = Vec::new();

    // Doing it again is worth noting, however little it changes the score.
    if mark.times > 1 && mark.status == Status::Yes {
        parts.push(small_text(
            format!("×{}", mark.times),
            cx,
            cy + 13.0,
            HC,
            9.0,
        ));
    }
    if let Some(value) = mark.value {
        parts.push(small_text(
            habits::format_amount(value),
            cx,
            cy - 16.0,
            HC,
            9.0,
        ));
    }
    // As is holding back. A day that slipped shows its slips beside however
    // many times it held back.
    match mark.status {
        Status::Clean if mark.resisted > 0 => {
            parts.push(small_text(
                format!("+{}", mark.resisted),
                cx,
                cy + 13.0,
                HC,
                9.0,
            ));
        }
        Status::Slipped(slips) => {
            let slips = slips.max(mark.times);
            if mark.resisted > 0 {
                parts.push(small_text(
                    format!("+{}", mark.resisted),
                    cx - 8.0,
                    cy + 13.0,
                    HC,
                    9.0,
                ));
                parts.push(small_text(
                    format!("+{slips}"),
                    cx + 8.0,
                    cy + 13.0,
                    SLIP,
                    9.0,
                ));
            } else {
                parts.push(small_text(format!("+{slips}"), cx, cy + 13.0, SLIP, 9.0));
            }
        }
        _ => {}
    }

    let face = (mark.icons == Icons::Faces).then(|| mark.face()).flatten();
    if let Some((face, tint, ink)) = face {
        parts.push(draw_face(cx, cy, face, tint, ink));
    } else {
        let cross = |color: &str, reach: f32, width: f32| {
            rsx! {
                {polyline(&at(&[(-reach, -reach), (reach, reach)]), color, width)}
                {polyline(&at(&[(reach, -reach), (-reach, reach)]), color, width)}
            }
        };
        parts.push(match mark.status {
            Status::Clean | Status::Yes => polyline(&at(CHECK), HC, 2.6),
            // Doing what was meant to be avoided is the one mark that stands out.
            Status::Slipped(_) => cross(SLIP, 4.5, 2.6),
            // Before a "No" habit began there is nothing to judge.
            Status::Unknown if mark.kind == Kind::No && !mark.question_marks => {
                polyline(&at(&[(-3.0, 0.0), (3.0, 0.0)]), QUIET, 2.0)
            }
            // Covered by an earlier repetition: Loop's quieter check.
            Status::Auto => polyline(&at(CHECK), QUIET, 2.2),
            Status::Partial(reps) => small_text(format!("{reps}/{}", mark.goal), cx, cy, HC, 12.0),
            Status::Skip => rsx! {
                {polyline(&at(&[(-5.5, -4.5), (-0.5, 0.0), (-5.5, 4.5)]), HC, 2.2)}
                {polyline(&at(&[(0.5, -4.5), (5.5, 0.0), (0.5, 4.5)]), HC, 2.2)}
            },
            Status::Unknown if mark.question_marks => {
                small_text("?".to_owned(), cx, cy, QUIET, 15.0)
            }
            Status::No | Status::Unknown => cross(QUIET, 4.0, 2.0),
        });
    }

    rsx! {
        svg {
            class: "habit-mark-glyph",
            width: "{width}",
            height: "{height}",
            view_box: "0 0 {width} {height}",
            "aria-hidden": "true",
            {parts.into_iter()}
        }
    }
}

/// An emoji-style face, drawn so it does not depend on the fonts installed.
fn draw_face(cx: f32, cy: f32, face: Face, tint: &str, ink: &str) -> Element {
    const RADIUS: f32 = 9.5;
    if face == Face::Question {
        return rsx! {
            circle { cx: "{fmt(cx)}", cy: "{fmt(cy)}", r: "{RADIUS}", style: "fill: {tint};" }
            {small_text("?".to_owned(), cx, cy, ink, 14.0)}
        };
    }
    // The mouth curves up for a smile and down for disappointment.
    let (ends, bend) = if face == Face::Happy {
        (1.8, 7.8)
    } else {
        (5.2, 1.0)
    };
    let mouth = format!(
        "M {} {} Q {} {} {} {}",
        fmt(cx - 4.2),
        fmt(cy + ends),
        fmt(cx),
        fmt(cy + bend),
        fmt(cx + 4.2),
        fmt(cy + ends)
    );
    rsx! {
        circle { cx: "{fmt(cx)}", cy: "{fmt(cy)}", r: "{RADIUS}", style: "fill: {tint};" }
        circle { cx: "{fmt(cx - 3.3)}", cy: "{fmt(cy - 2.4)}", r: "1.4", style: "fill: {ink};" }
        circle { cx: "{fmt(cx + 3.3)}", cy: "{fmt(cy - 2.4)}", r: "1.4", style: "fill: {ink};" }
        path { d: "{mouth}", style: "fill: none; stroke: {ink}; stroke-width: 1.6; stroke-linecap: round;" }
    }
}

// -------------------------------------------------------------- axis labels

/// Two lines under a column. Loop names the period, and adds the year whenever
/// it changes.
pub fn axis_label(
    date: NaiveDate,
    previous: Option<NaiveDate>,
    bucket: Bucket,
) -> (String, Option<String>) {
    let new_year = previous.is_none_or(|previous| previous.year() != date.year());
    let year = new_year.then(|| date.year().to_string());
    match bucket {
        Bucket::Day | Bucket::Week => {
            if previous.is_none_or(|previous| previous.month() != date.month()) {
                (month_short(date.month()).to_owned(), year)
            } else {
                (date.day().to_string(), None)
            }
        }
        Bucket::Month => (month_short(date.month()).to_owned(), year),
        Bucket::Quarter => (format!("Q{}", date.month0() / 3 + 1), year),
        Bucket::Year => (date.year().to_string(), None),
    }
}

fn label(content: impl Into<String>, x: f32, y: f32, color: &str, anchor: &str) -> Element {
    let content = content.into();
    rsx! {
        text {
            x: "{fmt(x)}",
            y: "{fmt(y)}",
            "text-anchor": "{anchor}",
            "dominant-baseline": "central",
            style: "fill: {color}; font-size: {TEXT_SIZE}px;",
            "{content}"
        }
    }
}

/// The labels under each column, oldest first.
fn axis(columns: &[(f32, NaiveDate)], bucket: Bucket, baseline: f32) -> Vec<Element> {
    let mut previous = None;
    let mut parts = Vec::new();
    for (x, date) in columns {
        let (first, second) = axis_label(*date, previous, bucket);
        parts.push(label(first, *x, baseline + 11.0, MEDIUM, "middle"));
        if let Some(second) = second {
            parts.push(label(second, *x, baseline + 24.0, MEDIUM, "middle"));
        }
        previous = Some(*date);
    }
    parts
}

/// Lines across the whole card, behind the part that scrolls.
fn grid(heights: &[f32], height: f32) -> Element {
    rsx! {
        svg {
            class: "habit-chart-grid",
            width: "100%",
            height: "{height}",
            view_box: "0 0 100 {height}",
            preserve_aspect_ratio: "none",
            "aria-hidden": "true",
            for y in heights.iter().copied() {
                line {
                    x1: "0",
                    x2: "100",
                    y1: "{fmt(y)}",
                    y2: "{fmt(y)}",
                    "vector-effect": "non-scaling-stroke",
                    style: "stroke: {LOW}; stroke-width: 1;",
                }
            }
        }
    }
}

/// A chart that scrolls sideways and opens on its newest, right-hand end.
/// `fixed` stays put over it: labels the desktop draws at the frame's edge.
fn scroller(
    height: f32,
    width: f32,
    background: Option<Element>,
    content: Element,
    fixed: Option<Element>,
    gutter: f32,
) -> Element {
    rsx! {
        div { class: "habit-chart", style: "height: {height}px;",
            {background}
            div { class: "habit-chart-scroll", style: "right: {gutter}px;",
                svg {
                    class: "habit-chart-plot",
                    width: "{fmt(width)}",
                    height: "{height}",
                    view_box: "0 0 {fmt(width)} {height}",
                    {content}
                }
            }
            {fixed}
        }
    }
}

// -------------------------------------------------------------------- score

/// Loop's score card: a line with a dot on each period, newest on the right.
#[component]
pub fn ScoreChart(series: Vec<(NaiveDate, f64)>, bucket: Bucket) -> Element {
    const HEIGHT: f32 = 220.0;
    let plot = HEIGHT - AXIS_HEIGHT;
    let y = |score: f64| 4.0 + (plot - 8.0) * (1.0 - score.clamp(0.0, 1.0) as f32);
    let count = series.len().max(MIN_COLUMNS);
    let width = count as f32 * COLUMN_WIDTH;
    // Oldest first, so the axis reads left to right.
    let columns: Vec<(f32, NaiveDate, f64)> = series
        .iter()
        .enumerate()
        .rev()
        .map(|(index, (date, score))| {
            let x = width - index as f32 * COLUMN_WIDTH - COLUMN_WIDTH / 2.0;
            (x, *date, *score)
        })
        .collect();
    let path: Vec<String> = columns
        .iter()
        .map(|(x, _, score)| format!("{},{}", fmt(*x), fmt(y(*score))))
        .collect();
    let path = path.join(" ");
    let steps: Vec<f32> = (0..=5).map(|step| y(f64::from(step) / 5.0)).collect();
    let dates: Vec<(f32, NaiveDate)> = columns.iter().map(|(x, date, _)| (*x, *date)).collect();
    let axis = axis(&dates, bucket, plot);
    let percents = rsx! {
        svg { class: "habit-chart-fixed", width: "100%", height: "{HEIGHT}", "aria-hidden": "true",
            for step in 1..=5 {
                {label(format!("{}%", step * 20), 4.0, y(f64::from(step) / 5.0) + 9.0, MEDIUM, "start")}
            }
        }
    };
    let content = rsx! {
        polyline { points: "{path}", style: "fill: none; stroke: {HC}; stroke-width: 2;" }
        for (x, date, score) in columns.iter().copied() {
            circle { key: "{date}", cx: "{fmt(x)}", cy: "{fmt(y(score))}", r: "3.5", style: "fill: {HC};" }
        }
        {axis.into_iter()}
    };
    scroller(
        HEIGHT,
        width,
        Some(grid(&steps, HEIGHT)),
        content,
        Some(percents),
        0.0,
    )
}

// ------------------------------------------------------------------ history

/// Loop's history card: what was counted per period as labelled bars. With a
/// target, it is the Value chart, with the target as a line across.
#[component]
pub fn HistoryChart(series: Vec<(NaiveDate, f64)>, bucket: Bucket, target: Option<f64>) -> Element {
    const HEIGHT: f32 = 200.0;
    const TOP: f32 = 16.0;
    let plot = HEIGHT - AXIS_HEIGHT;
    let tallest = (plot - TOP).max(1.0);
    let count = series.len().max(MIN_COLUMNS);
    let width = count as f32 * COLUMN_WIDTH;
    let most = series
        .iter()
        .map(|(_, value)| *value)
        .chain(target)
        .fold(0.0_f64, f64::max);
    let most = if most > 0.0 { most } else { 1.0 };
    let mut dates = Vec::new();
    let mut bars = Vec::new();
    for (index, (date, value)) in series.iter().enumerate().rev() {
        let x = width - index as f32 * COLUMN_WIDTH - COLUMN_WIDTH / 2.0;
        dates.push((x, *date));
        if *value <= 0.0 {
            continue;
        }
        let height = (tallest * (*value / most) as f32).max(1.0);
        let top = plot - height;
        // Rounded at the top only, as on the desktop.
        let bar = format!(
            "M {left} {plot} V {top_r} Q {left} {top} {left_r} {top} H {right_r} Q {right} {top} {right} {top_r} V {plot} Z",
            left = fmt(x - 7.0),
            right = fmt(x + 7.0),
            left_r = fmt(x - 5.0),
            right_r = fmt(x + 5.0),
            top = fmt(top),
            top_r = fmt(top + 2.0_f32.min(height)),
            plot = fmt(plot),
        );
        bars.push(rsx! {
            path { key: "{date}", d: "{bar}", style: "fill: {HC};" }
            {label(habits::format_amount(*value), x, top - 8.0, HC, "middle")}
        });
    }
    let lines: Vec<f32> = (0..5)
        .map(|step| TOP + tallest * step as f32 / 4.0)
        .collect();
    let axis = axis(&dates, bucket, plot);
    let target_line = target.map(|target| {
        let y = plot - tallest * (target / most) as f32;
        let words = format!("Target {}", habits::format_amount(target));
        let patch = words.chars().count() as f32 * 6.5 + 4.0;
        rsx! {
            svg { class: "habit-chart-fixed", width: "100%", height: "{HEIGHT}", "aria-hidden": "true",
                line {
                    x1: "0",
                    x2: "100%",
                    y1: "{fmt(y)}",
                    y2: "{fmt(y)}",
                    style: "stroke: {MEDIUM}; stroke-width: 1.5; stroke-dasharray: 5 4;",
                }
                // On a patch of the card, so that it stays readable over a bar.
                rect { x: "0", y: "{fmt(y - 15.0)}", width: "{fmt(patch)}", height: "14", style: "fill: {CARD};" }
                {label(words, 2.0, y - 8.0, MEDIUM, "start")}
            }
        }
    });
    let content = rsx! {
        {bars.into_iter()}
        {axis.into_iter()}
    };
    scroller(
        HEIGHT,
        width,
        Some(grid(&lines, HEIGHT)),
        content,
        target_line,
        0.0,
    )
}

// ----------------------------------------------------------------- calendar

const SQUARE: f32 = 26.0;
const SQUARE_GAP: f32 = 2.0;
const WEEKDAY_COLUMN: f32 = 34.0;
/// Weeks drawn however short the history, enough for a phone's width.
const MIN_WEEKS: i64 = 12;

/// Loop's calendar card: one square per day, in a column per week, ending at
/// today. Tapping a square opens that day, as on the list.
#[component]
pub fn CalendarChart(
    computed: Computed,
    today: NaiveDate,
    on_day: EventHandler<NaiveDate>,
) -> Element {
    let height = SQUARE * 8.0;
    let this_week = Bucket::Week.start(today, FIRST_WEEKDAY);
    // Weeks back to the first record, and no further.
    let weeks = ((today - computed.first_day()).num_days() / 7 + 1).max(MIN_WEEKS);
    let first = this_week - Duration::days(7 * (weeks - 1));
    let size = SQUARE - SQUARE_GAP;
    let dimmed = mix(HC, 0.5, CARD);
    let width = weeks as f32 * SQUARE;

    let mut parts = Vec::new();
    let mut last_month = None;
    let mut last_year = None;
    // Text may run past its own column, so the next one starts after it.
    let mut overflow: f32 = 0.0;
    for column in 0..weeks {
        let week = first + Duration::days(column * 7);
        let x = column as f32 * SQUARE;
        let header = if last_month != Some(week.month()) {
            last_month = Some(week.month());
            month_short(week.month()).to_owned()
        } else if last_year != Some(week.year()) {
            last_year = Some(week.year());
            week.year().to_string()
        } else {
            String::new()
        };
        if !header.is_empty() {
            parts.push(label(
                header.clone(),
                x + overflow,
                SQUARE / 2.0,
                MEDIUM,
                "start",
            ));
        }
        overflow += header.len() as f32 * TEXT_SIZE * 0.55 + SQUARE * 0.1;
        overflow = (overflow - SQUARE).max(0.0);

        for row in 0..7 {
            let day = week + Duration::days(row);
            if day > today {
                continue;
            }
            let status = computed.status(day);
            // Loop's squares: on, and dimmed when covered.
            let (fill, on) = match status {
                Status::Yes | Status::Clean => (HC.to_owned(), true),
                Status::Auto | Status::Skip | Status::Partial(_) => (dimmed.clone(), false),
                Status::No | Status::Unknown | Status::Slipped(_) => (LOW.to_owned(), false),
            };
            let (left, top) = (x, SQUARE + row as f32 * SQUARE);
            let hatching = (status == Status::Skip).then(|| {
                // Loop's hatching, in the card's color.
                let mut lines = Vec::new();
                let mut k = size / 10.0;
                for _ in 0..5 {
                    lines.push((left + k, top, left, top + k));
                    lines.push((left + size - k, top + size, left + size, top + size - k));
                    k += size / 5.0;
                }
                rsx! {
                    for (x1, y1, x2, y2) in lines {
                        line {
                            x1: "{fmt(x1)}",
                            y1: "{fmt(y1)}",
                            x2: "{fmt(x2)}",
                            y2: "{fmt(y2)}",
                            style: "stroke: {CARD}; stroke-width: 0.75; pointer-events: none;",
                        }
                    }
                }
            });
            let text = if on { CARD } else { "var(--habit-day-text)" };
            parts.push(rsx! {
                g { key: "{day}", class: "habit-calendar-day",
                    onclick: move |_| on_day.call(day),
                    rect {
                        x: "{fmt(left)}",
                        y: "{fmt(top)}",
                        width: "{fmt(size)}",
                        height: "{fmt(size)}",
                        rx: "{fmt(size * 0.15)}",
                        style: "fill: {fill};",
                    }
                    {hatching}
                    text {
                        x: "{fmt(left + size / 2.0)}",
                        y: "{fmt(top + size / 2.0)}",
                        "text-anchor": "middle",
                        "dominant-baseline": "central",
                        style: "fill: {text}; font-size: {TEXT_SIZE}px; pointer-events: none;",
                        "{day.day()}"
                    }
                }
            });
        }
    }
    let names = rsx! {
        svg { class: "habit-chart-fixed habit-chart-names", width: "{WEEKDAY_COLUMN}", height: "{height}", "aria-hidden": "true",
            for (row, name) in WEEKDAYS.iter().enumerate() {
                {label(*name, 2.0, SQUARE + row as f32 * SQUARE + size / 2.0, MEDIUM, "start")}
            }
        }
    };
    scroller(
        height,
        width,
        None,
        rsx! { {parts.into_iter()} },
        Some(names),
        WEEKDAY_COLUMN,
    )
}

// ------------------------------------------------------------------ streaks

/// Loop's best streaks card: centred bars with the dates on either side.
#[component]
pub fn StreakChart(streaks: Vec<Streak>) -> Element {
    let longest = streaks
        .iter()
        .map(|streak| streak.length())
        .max()
        .unwrap_or(1)
        .max(1);
    let short_date =
        |day: NaiveDate| format!("{} {}, {}", month_short(day.month()), day.day(), day.year());
    rsx! {
        div { class: "habit-streaks",
            for streak in streaks {
                {
                    let share = streak.length() as f32 / longest as f32;
                    let (fill, ink) = if share >= 1.0 {
                        (HC.to_owned(), CARD)
                    } else if share >= 0.8 {
                        (mix(HC, 0.75, CARD), CARD)
                    } else if share >= 0.5 {
                        (mix(HC, 0.38, CARD), CARD)
                    } else {
                        (LOW.to_owned(), MEDIUM)
                    };
                    rsx! {
                        div { key: "{streak.start}", class: "habit-streak",
                            span { class: "habit-streak-date start", "{short_date(streak.start)}" }
                            span {
                                class: "habit-streak-bar",
                                style: "--share: {share:.3}; background: {fill}; color: {ink};",
                                "{streak.length()}"
                            }
                            span { class: "habit-streak-date end", "{short_date(streak.end)}" }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- frequency

const FREQUENCY_CELL: f32 = 30.0;
/// Months drawn however short the history.
const MIN_MONTHS: u32 = 10;

/// Loop's frequency card: a dot per weekday per month, sized by how often the
/// habit was done on that weekday.
#[component]
pub fn FrequencyChart(computed: Computed, today: NaiveDate) -> Element {
    let height = FREQUENCY_CELL * 7.0 + AXIS_HEIGHT;
    let first_weekday = FIRST_WEEKDAY.num_days_from_monday() as usize;
    let data = habits::weekday_frequency(&computed);
    let this_month = today.with_day(1).unwrap_or(today);
    let oldest = data.keys().next().copied().unwrap_or(this_month);
    let span = (this_month.year() - oldest.year()) * 12 + this_month.month() as i32
        - oldest.month() as i32
        + 1;
    let months = (span.max(1) as u32).max(MIN_MONTHS);
    let first = this_month
        .checked_sub_months(chrono::Months::new(months - 1))
        .unwrap_or(this_month);
    let width = months as f32 * FREQUENCY_CELL;
    let shades = [
        LOW.to_owned(),
        mix(HC, 0.33, LOW),
        mix(HC, 0.66, LOW),
        HC.to_owned(),
    ];

    let mut dates = Vec::new();
    let mut dots = Vec::new();
    for column in 0..months {
        let month = first
            .checked_add_months(chrono::Months::new(column))
            .unwrap_or(first);
        let x = column as f32 * FREQUENCY_CELL + FREQUENCY_CELL / 2.0;
        dates.push((x, month));
        let Some(values) = data.get(&month) else {
            continue;
        };
        let available = habits::weekdays_in_month(month);
        for row in 0..7 {
            let weekday = (row + first_weekday) % 7;
            let out_of = f64::from(available[weekday].max(1));
            let scale = (values[weekday] / out_of).min(1.0) as f32;
            if scale <= 0.0 {
                continue;
            }
            let radius = (FREQUENCY_CELL - 2.0 * FREQUENCY_CELL * 0.2) / 2.0 * scale;
            let shade = shades[((shades.len() - 1) as f32 * scale).round() as usize].clone();
            dots.push(rsx! {
                circle {
                    key: "{month}-{row}",
                    cx: "{fmt(x)}",
                    cy: "{fmt(row as f32 * FREQUENCY_CELL + FREQUENCY_CELL / 2.0)}",
                    r: "{fmt(radius)}",
                    style: "fill: {shade};",
                }
            });
        }
    }
    let lines: Vec<f32> = (0..=7).map(|row| row as f32 * FREQUENCY_CELL).collect();
    let axis = axis(&dates, Bucket::Month, FREQUENCY_CELL * 7.0);
    let names = rsx! {
        svg { class: "habit-chart-fixed habit-chart-names", width: "{WEEKDAY_COLUMN}", height: "{height}", "aria-hidden": "true",
            for (row, name) in WEEKDAYS.iter().enumerate() {
                {label(*name, 2.0, row as f32 * FREQUENCY_CELL + FREQUENCY_CELL / 2.0, MEDIUM, "start")}
            }
        }
    };
    let content = rsx! {
        {dots.into_iter()}
        {axis.into_iter()}
    };
    scroller(
        height,
        width,
        Some(grid(&lines, height)),
        content,
        Some(names),
        WEEKDAY_COLUMN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn axis_labels_name_the_month_once_and_the_year_when_it_changes() {
        let (first, year) = axis_label(on(2026, 1, 5), None, Bucket::Week);
        assert_eq!((first.as_str(), year.as_deref()), ("Jan", Some("2026")));
        let (next, year) = axis_label(on(2026, 1, 12), Some(on(2026, 1, 5)), Bucket::Week);
        assert_eq!((next.as_str(), year), ("12", None));
        let (quarter, _) = axis_label(on(2026, 4, 1), Some(on(2026, 1, 1)), Bucket::Quarter);
        assert_eq!(quarter, "Q2");
    }

    #[test]
    fn a_habit_color_is_one_of_the_palette_variables() {
        assert_eq!(
            habit_color(3),
            "--hc: var(--habit-3); --hc-ink: var(--habit-ink-3);"
        );
        assert!(habit_color(habits::PALETTE_SIZE + 1).contains("--habit-1)"));
    }
}
