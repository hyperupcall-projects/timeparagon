//! Moving between screens.

use super::*;

pub(super) fn update(state: &mut State, message: NavigationMessage) -> Task<Message> {
    match message {
        NavigationMessage::PlanTabSelected(tab) => {
            if tab == PlanTab::Schedule && !state.range_view_active() {
                return Task::none();
            }
            select_screen(
                state,
                match tab {
                    PlanTab::Schedule => Screen::Calendar,
                    PlanTab::Overdue => Screen::Overdue,
                    PlanTab::Inbox => Screen::Inbox,
                    PlanTab::Today => Screen::Today,
                    PlanTab::Backlog => Screen::Backlog,
                },
            )
        }
        NavigationMessage::PlanDaySelected => {
            state.plan_view = CalendarView::Day;
            let was_planning = state.range_view_active();
            state.view = CalendarView::Day;
            let close = range_view_transition(state, was_planning, Task::none());
            let screen = match state.screen {
                Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Backlog => state.screen,
                _ => Screen::Overdue,
            };
            Task::batch([
                close,
                select_screen(state, screen),
                show_cached_calendar(state),
                ui::calendar::scroll_to_now(
                    state.calendar.hour_height,
                    state.config.calendar.day_end_minutes(),
                ),
            ])
        }
        NavigationMessage::ViewMenuToggled => {
            state.view_menu_open = !state.view_menu_open;
            // Only one dropdown is open at a time.
            if state.view_menu_open {
                state.more_menu_open = false;
            }
            Task::none()
        }
        NavigationMessage::MoreMenuToggled => {
            state.more_menu_open = !state.more_menu_open;
            if state.more_menu_open {
                state.view_menu_open = false;
                state.activity.open = false;
            }
            Task::none()
        }
        NavigationMessage::ActivityLogToggled => {
            state.activity.open = !state.activity.open;
            if state.activity.open {
                state.view_menu_open = false;
                state.more_menu_open = false;
            }
            Task::none()
        }
        NavigationMessage::ScreenSelected(screen) => {
            let was_planning = state.range_view_active();
            state.view = CalendarView::Day;
            let close = range_view_transition(state, was_planning, Task::none());
            Task::batch([close, select_screen(state, screen)])
        }
    }
}

/// Change sidebar content without changing the planning range.
pub(super) fn select_screen(state: &mut State, screen: Screen) -> Task<Message> {
    state.view_layout_input = None;
    // Choosing a destination puts the dropdown menus away.
    state.view_menu_open = false;
    state.more_menu_open = false;
    state.activity.open = false;
    // Alarm setting belongs to the day view, so leaving it ends both.
    state.alarms.mode = false;
    state.alarms.editor = None;
    // Keep the day grid's scroll position when switching screens.
    state.calendar.menu_open = false;
    state.calendar.search.open = false;
    state.calendar.actions_open = false;
    // Each task list opens its own selection in the details pane.
    let tasks_screen = |screen| {
        matches!(
            screen,
            Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Triaged | Screen::Backlog
        )
    };
    if screen != state.screen && (tasks_screen(screen) || tasks_screen(state.screen)) {
        state.calendar.open_event = None;
        state.inbox.picker = None;
    }
    if screen == Screen::Habits {
        state.calendar.open_event = None;
        state.habits.confirm_delete = None;
    } else {
        state.habits.menu = None;
        state.habits.section_edit = None;
    }
    if screen != Screen::Habits {
        state.habits.calendar_habit = None;
    }
    let left_notes = if state.screen == Screen::Notes && screen != Screen::Notes {
        crate::notes::leave(state)
    } else {
        Task::none()
    };
    state.screen = screen;
    if screen != Screen::BrainDump {
        state.brain_dump.open = None;
    }
    let close = crate::timeblocking::close(state);
    let close = Task::batch([close, left_notes]);
    if screen == Screen::Notes {
        Task::batch([close, crate::notes::refresh(state)])
    } else if screen == Screen::People {
        Task::batch([close, crate::people::refresh(state)])
    } else if screen == Screen::Habits {
        Task::batch([
            close,
            Task::done(Message::Habits(crate::habits::Action::Refresh)),
        ])
    } else {
        close
    }
}
