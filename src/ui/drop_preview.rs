//! Shows where the task in the air would land if it were let go now.
//!
//! The drop under the pointer is found by [`super::task_drop::probe`], which
//! runs the plan view's own release handling, and kept in
//! `state.timeblocking.drop_hover`. These helpers turn it into the task as it
//! would be after the drop, for each view to draw in place.
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::timeblocking::{self as tb, Action, DropHover};
use chrono::NaiveDate;
use iced::widget::{container, text};
use iced::{Element, Fill, Theme};

fn hover(state: &State) -> Option<(&DropHover, &Event)> {
    let id = state.timeblocking.dragging.as_deref()?;
    let hover = state.timeblocking.drop_hover.as_ref()?;
    Some((hover, state.calendar.event(id)?))
}

/// The day a hover stands for: its column's, or the day being planned.
fn day_of(state: &State, hover: &DropHover) -> NaiveDate {
    hover.day.unwrap_or(state.calendar.day)
}

/// The dragged task as it would be timed after a drop on the time grid,
/// matching [`Action::DropTime`].
pub(crate) fn timed(state: &State) -> Option<Event> {
    let (hover, task) = hover(state)?;
    let Action::DropTime(start) = hover.action else {
        return None;
    };
    let minutes = match tb::metadata(task).minutes {
        0 => 30,
        minutes => minutes,
    };
    let mut task = task.clone();
    task.start = start;
    task.end = start + chrono::Duration::minutes(i64::from(minutes));
    task.all_day = false;
    Some(task)
}

/// Whether letting go now drops the task on `day` as a whole, matching
/// [`Action::DropDay`], which makes it an all-day task on that day.
pub(crate) fn on_day(state: &State, day: NaiveDate) -> Option<Event> {
    let (hover, task) = hover(state)?;
    if !matches!(hover.action, Action::DropDay) || day_of(state, hover) != day {
        return None;
    }
    let mut task = task.clone();
    task.start = calendar::at_clock(day, 0, 0);
    task.end = task.start + chrono::Duration::days(1);
    task.all_day = true;
    Some(task)
}

/// Where a drop on the week board's `day` column would put the task: the card
/// it goes next to (none for the end of the column), whether it goes after
/// that card, and the task as it would then be timed.
pub(crate) fn week_list(state: &State, day: NaiveDate) -> Option<(Option<String>, bool, Event)> {
    let (hover, task) = hover(state)?;
    let Action::DropWeekList { target, after } = &hover.action else {
        return None;
    };
    if day_of(state, hover) != day || target.as_deref() == Some(task.id.as_str()) {
        return None;
    }
    let boundary = state.config.calendar.day_start_hour();
    let mut preview = task.clone();
    // Matches `drop_week_list`: a task already timed on the day keeps its
    // time and only moves in the list.
    if task.all_day || calendar::day_at(task.start, boundary) != day {
        let (start, end) = tb::week_drop_times(task, day, boundary)?;
        preview.start = start;
        preview.end = end;
        preview.all_day = false;
    }
    Some((target.clone(), *after, preview))
}

/// The time block the task would go into, or the task row in a block it
/// would be dropped on (and so join that row's block).
pub(crate) fn block_or_row(state: &State) -> Option<&str> {
    let (hover, _) = hover(state)?;
    match &hover.action {
        Action::DropBlock(block) => Some(block),
        // On the Backlog screen a row drop sorts into a backlog section instead.
        Action::DropTask(row) if state.screen != crate::app::Screen::Backlog => Some(row),
        _ => None,
    }
}

/// Whether the floating card should step aside: on the time grid the preview
/// itself sits under the pointer and shows the task better.
pub(crate) fn hides_ghost(state: &State) -> bool {
    state
        .timeblocking
        .drop_hover
        .as_ref()
        .is_some_and(|hover| matches!(hover.action, Action::DropTime(_)))
}

/// The outline drawn around a preview, so it reads as where the task would go
/// rather than as something already there.
pub(crate) fn outline(theme: &Theme) -> iced::Border {
    iced::Border {
        color: theme.extended_palette().primary.strong.color,
        width: 2.0,
        radius: super::controls::RADIUS_ROW.into(),
    }
}

/// A one-line chip for the dragged task in its own color, for the day cells
/// and all-day rows.
pub(crate) fn chip<'a>(state: &State, task: &Event, size: f32) -> Element<'a, Message> {
    let color = task.display_color(&state.config.calendar);
    let ink = calendar::colors::on_color(color);
    container(
        text(tb::task_label(task))
            .size(size)
            .color(ink)
            .wrapping(text::Wrapping::None),
    )
    .width(Fill)
    .padding([3, 6])
    .clip(true)
    .style(move |theme: &Theme| container::Style {
        background: Some(color.into()),
        border: outline(theme),
        shadow: iced::Shadow {
            color: iced::Color::BLACK.scale_alpha(0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 6.0,
        },
        ..container::Style::default()
    })
    .into()
}
