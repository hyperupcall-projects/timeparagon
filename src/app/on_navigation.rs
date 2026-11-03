//! Moving between screens.

use super::*;

pub(super) fn update(state: &mut State, message: NavigationMessage) -> Task<Message> {
    match message {
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
            }
            Task::none()
        }
        NavigationMessage::ScreenSelected(screen) => {
            state.view_layout_input = None;
            // Choosing a destination puts the dropdown menus away.
            state.view_menu_open = false;
            state.more_menu_open = false;
            // Alarm setting belongs to the day view, so leaving it ends both.
            state.alarms.mode = false;
            state.alarms.editor = None;
            // Keep the day grid's scroll position when switching screens.
            state.calendar.menu_open = false;
            state.calendar.search.open = false;
            // Each task list opens its own selection in the details pane.
            let tasks_screen =
                |screen| matches!(screen, Screen::Overdue | Screen::Inbox | Screen::Triaged);
            if screen != state.screen && (tasks_screen(screen) || tasks_screen(state.screen)) {
                state.calendar.open_event = None;
                state.inbox.picker = None;
            }
            if screen == Screen::Habits {
                state.calendar.open_event = None;
                state.habits.confirm_delete = None;
            }
            if screen != Screen::Habits {
                state.habits.calendar_habit = None;
            }
            state.screen = screen;
            if screen != Screen::BrainDump {
                state.brain_dump.open = None;
            }
            let close = crate::timeblocking::close(state);
            if screen == Screen::KeepNotes {
                Task::batch([close, refresh_keep(state)])
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
    }
}
