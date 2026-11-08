//! What the calendar shows and keeping it in sync.

use super::*;

pub(super) fn update(state: &mut State, message: CalendarMessage) -> Task<Message> {
    match message {
        CalendarMessage::CalendarTaskListToggled(show) => {
            state.config.calendar.weekly_task_list = show;
            state.week_scroll_x = 0.0;
            let scroll = if show {
                ui::calendar::scroll_week_to_start()
            } else {
                ui::calendar::scroll_week_to_now(
                    state.calendar.hour_height,
                    state.config.calendar.day_end_minutes(),
                )
            };
            Task::batch([
                queue_save(state),
                ui::calendar::scroll_week_to_start(),
                scroll,
            ])
        }
        CalendarMessage::CalendarModeSelected(weekly) => {
            let navigation = if weekly {
                planning_sidebar(state)
            } else {
                super::on_navigation::select_screen(state, Screen::Calendar)
            };
            state.view_menu_open = false;
            state.habits.calendar_habit = None;
            let was_weekly = state.range_view_active();
            let view = if weekly {
                CalendarView::Week
            } else {
                CalendarView::Day
            };
            let changed = state.view != view;
            state.week_hover = None;
            state.view = view;
            let close = range_view_transition(state, was_weekly, Task::none());
            if changed {
                state.week_anchor = state.calendar.day;
                state.week_scroll_x = 0.0;
                let scroll = if weekly {
                    ui::calendar::scroll_week_to_now(
                        state.calendar.hour_height,
                        state.config.calendar.day_end_minutes(),
                    )
                } else {
                    Task::none()
                };
                Task::batch([navigation, close, scroll, show_cached_calendar(state)])
            } else {
                Task::batch([navigation, close])
            }
        }
        CalendarMessage::CalendarMonthViewSelected => {
            let navigation = planning_sidebar(state);
            state.view_menu_open = false;
            let was_wide = state.range_view_active();
            let changed = !state.monthly();
            state.week_hover = None;
            state.view = CalendarView::Month;
            if changed {
                state.week_anchor = state.calendar.day;
            }
            let resize = range_view_transition(state, was_wide, Task::none());
            Task::batch([navigation, resize, show_cached_calendar(state)])
        }
        CalendarMessage::CalendarTwoWeekViewSelected => {
            let navigation = planning_sidebar(state);
            state.view_menu_open = false;
            state.habits.calendar_habit = None;
            let was_wide = state.range_view_active();
            let changed = !state.two_weekly();
            state.week_hover = None;
            state.two_week_more = None;
            state.view = CalendarView::TwoWeek;
            if changed {
                state.week_anchor = state.calendar.day;
            }
            let resize = range_view_transition(state, was_wide, Task::none());
            Task::batch([navigation, resize, show_cached_calendar(state)])
        }
        CalendarMessage::CalendarTwoWeekMore(day) => {
            state.two_week_more = day;
            Task::none()
        }
        CalendarMessage::CalendarTwoWeekCellPressed(day) => {
            ui::calendar::anchor_draft_at_pointer();
            let hour = state.config.calendar.day_start_hour().max(9);
            let start = calendar::at_clock(day, hour, 0);
            dispatch(
                state,
                Message::Calendar(CalendarMessage::WeekDayAction(
                    day,
                    Box::new(Message::EventDraft(EventDraftMessage::EventDraftStarted {
                        start,
                        end: start + chrono::Duration::hours(1),
                    })),
                )),
            )
        }
        CalendarMessage::CalendarViewMonthShift(months) => {
            let first = calendar::first_of_month(state.week_anchor);
            let step = chrono::Months::new(months.unsigned_abs());
            let day = if months >= 0 {
                first.checked_add_months(step)
            } else {
                first.checked_sub_months(step)
            };
            if let Some(day) = day {
                return dispatch(
                    state,
                    Message::Calendar(CalendarMessage::CalendarDaySelected(day)),
                );
            }
            Task::none()
        }
        CalendarMessage::CalendarDayOpened(day) => {
            let today =
                calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, today);
            let close = crate::timeblocking::close(state);
            Task::batch([close, show_cached_calendar(state)])
        }
        CalendarMessage::WeekHover(day) => {
            state.week_hover = day;
            Task::none()
        }
        CalendarMessage::WeekScrolled(x) => {
            state.week_scroll_x = x;
            Task::none()
        }
        CalendarMessage::WeekDayAction(day, message) => {
            let message = Box::new(match *message {
                Message::TimeBlocking(crate::timeblocking::Action::DropTask(target))
                    if state.timeblocking.drag_window.or(state.main_window)
                        == state.main_window =>
                {
                    Message::TimeBlocking(crate::timeblocking::Action::DropCalendarTask(target))
                }
                message => message,
            });
            let selects_day = matches!(
                &*message,
                Message::Event(EventMessage::EventOpened(_))
                    | Message::Event(EventMessage::EventTimeChanged { .. })
                    | Message::Task(TaskMessage::TaskToggled(_))
                    | Message::Habits(crate::habits::Action::ToggleCompleted(_))
                    | Message::EventDraft(EventDraftMessage::EventDraftStarted { .. })
                    | Message::TimeBlocking(
                        crate::timeblocking::Action::View(_)
                            | crate::timeblocking::Action::Plan(_)
                            | crate::timeblocking::Action::NewWeekTask
                            | crate::timeblocking::Action::DropWeekList { .. }
                            | crate::timeblocking::Action::DropDay
                            | crate::timeblocking::Action::DropTime(_)
                            | crate::timeblocking::Action::DropTask(_)
                            | crate::timeblocking::Action::DropCalendarTask(_)
                            | crate::timeblocking::Action::DropBlock(_)
                    )
            );
            if selects_day && day != state.calendar.day {
                let today =
                    calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
                let follow_today = state.calendar.follow_today;
                state.calendar.set_day(day, today);
                state.calendar.follow_today = follow_today;
                if matches!(
                    &*message,
                    Message::Event(EventMessage::EventOpened(_))
                        | Message::TimeBlocking(
                            crate::timeblocking::Action::View(_)
                                | crate::timeblocking::Action::Plan(_)
                        )
                ) {
                    let _ = crate::timeblocking::close(state);
                }
            }
            dispatch(state, *message)
        }
        CalendarMessage::CalendarSearch(action) => {
            use calendar::{projects, search};
            if let search::Action::Projects(projects::Action::Add(id)) = &action
                && state.config.projects.get(*id).is_none()
            {
                return Task::none();
            }
            let toggled = matches!(action, search::Action::Toggle);
            let hiding = matches!(action, search::Action::DetailsByVisibility(true));
            state.calendar.search.update(action);
            if hiding {
                // A pane left open would go on showing what was just hidden.
                let open = state.calendar.open_event.clone();
                let close_event = open
                    .and_then(|id| state.calendar.event(&id).cloned())
                    .is_some_and(|event| state.calendar.search.hides_details(&event));
                let closed = close_event
                    .then(|| crate::app::update(state, Message::Event(EventMessage::EventClosed)));
                return Task::batch(
                    closed
                        .into_iter()
                        .chain([crate::timeblocking::close(state)]),
                );
            }
            if !(toggled && state.calendar.search.open) {
                return Task::none();
            }
            // One sheet at a time, the same as the menu and the month pickers.
            state.calendar.menu_open = false;
            state.calendar.color_editor = None;
            state.calendar.month_open = false;
            state.overview_month_open = false;
            iced::widget::operation::focus(iced::widget::Id::new(ui::calendar::SEARCH_INPUT))
        }
        CalendarMessage::CalendarMenuToggled => {
            state.calendar.menu_open = !state.calendar.menu_open;
            if state.calendar.menu_open {
                state.calendar.search.open = false;
            }
            // Dismissing the menu also puts away any open color picker, so it
            // does not pop back up the next time the menu is opened.
            state.calendar.color_editor = None;
            if state.calendar.menu_open {
                // The old single TimeSlay toggle is replaced by per-kind
                // toggles. Preserve its hidden state without leaving an
                // invisible calendar-wide switch behind the new controls.
                let hidden_app_ids: Vec<_> = state
                    .config
                    .calendar
                    .disabled
                    .iter()
                    .filter(|id| {
                        state.config.calendar.app_calendar.as_ref() == Some(id)
                            || state.calendar.calendars.iter().any(|calendar| {
                                &calendar.id == *id
                                    && calendar.is_app_calendar(&state.config.calendar)
                            })
                    })
                    .cloned()
                    .collect();
                if !hidden_app_ids.is_empty() {
                    for source in calendar::sources::TimeSlaySource::ALL {
                        if source != calendar::sources::TimeSlaySource::Sleep {
                            state
                                .config
                                .calendar
                                .set_calendar_enabled(source.id(), false);
                        }
                    }
                    for id in hidden_app_ids {
                        state.config.calendar.set_calendar_enabled(&id, true);
                    }
                    return queue_save(state);
                }
            }
            Task::none()
        }
        CalendarMessage::CalendarToggled(id, enabled) => {
            calendar::set_source_enabled(
                &mut state.config.calendar,
                &state.calendar.calendars,
                &id,
                enabled,
            );
            queue_save(state)
        }
        CalendarMessage::CalendarSourceMoved(id, up) => {
            let mut order: Vec<_> = state
                .calendar
                .display_sources(&state.config.calendar)
                .into_iter()
                .map(|source| source.id)
                .collect();
            let Some(index) = order.iter().position(|source| source == &id) else {
                return Task::none();
            };
            let next = if up {
                index.checked_sub(1)
            } else {
                index.checked_add(1).filter(|&next| next < order.len())
            };
            let Some(next) = next else {
                return Task::none();
            };
            order.swap(index, next);
            // Keep preferences for calendars temporarily absent from the account.
            for saved in &state.config.calendar.source_order {
                if !order.contains(saved) {
                    order.push(saved.clone());
                }
            }
            state.config.calendar.source_order = order;
            queue_save(state)
        }
        CalendarMessage::CalendarSyncRequested => sync_calendar(state),
        CalendarMessage::CalendarAutoSyncRequested => {
            Task::batch([auto_sync_calendar(state), crate::notes::refresh(state)])
        }
        CalendarMessage::CalendarResyncRequested => {
            // Days parked in memory and on disk would paint before the next
            // visit fetched them. Drop them so the resync is the source of
            // truth. The day in view stays up until its own fetch lands.
            state.calendar.invalidate_other_days();
            sync_calendar(state)
        }
        CalendarMessage::CalendarSyncedAt(revision, result) => {
            if revision != state.timeblocking.revision {
                state.calendar.sync = SyncState::Idle;
                return Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested));
            }
            Task::done(Message::Calendar(CalendarMessage::CalendarSynced(result)))
        }
        CalendarMessage::CalendarRangeSynced(revision, dates, result) => {
            if revision != state.timeblocking.revision {
                state.calendar.sync = SyncState::Idle;
                return Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested));
            }
            match result {
                Ok(mut snapshot) => {
                    state
                        .activity
                        .record(crate::activity::synced_text(&snapshot.events), false);
                    if let Some(notice) = state.timeblocking.sync_notice.take() {
                        snapshot.notice = Some(match snapshot.notice.take() {
                            Some(existing) => format!("{notice}\n{existing}"),
                            None => notice,
                        });
                    }
                    let remember = remember_app_calendar(state, &snapshot);
                    state.calendar.cache_fetched_snapshot(snapshot, &dates);
                    state.calendar.apply_task_colors(&state.config.calendar);
                    let reconcile = reconcile_completed_task(state);
                    let fetch = fetch_pending_calendar_days(state, &dates);
                    // A new recurring series may still need adopting into the
                    // habit library before its occurrences can offer Record.
                    let habits = Task::done(Message::Habits(crate::habits::Action::Refresh));
                    let reconcile = Task::batch([reconcile, fetch, habits]);
                    if remember {
                        Task::batch([queue_save(state), reconcile])
                    } else {
                        reconcile
                    }
                }
                Err(error) => dispatch(
                    state,
                    Message::Calendar(CalendarMessage::CalendarSynced(Err(error))),
                ),
            }
        }
        CalendarMessage::CalendarSynced(Ok(mut snapshot)) => {
            state
                .activity
                .record(crate::activity::synced_text(&snapshot.events), false);
            if let Some(notice) = state.timeblocking.sync_notice.take() {
                snapshot.notice = Some(match snapshot.notice.take() {
                    Some(existing) => format!("{notice}\n{existing}"),
                    None => notice,
                });
            }
            let remember = remember_app_calendar(state, &snapshot);
            let day = snapshot.day;
            state.calendar.cache_fetched_snapshot(snapshot, &[day]);
            state.calendar.apply_task_colors(&state.config.calendar);
            let reconcile = reconcile_completed_task(state);
            let fetch = fetch_pending_calendar_days(state, &[day]);
            let invitations = crate::inbox::refresh_invitations(state);
            let habits = Task::done(Message::Habits(crate::habits::Action::Refresh));
            let reconcile = Task::batch([reconcile, fetch, invitations, habits]);
            if remember {
                Task::batch([queue_save(state), reconcile])
            } else {
                reconcile
            }
        }
        CalendarMessage::CalendarSynced(Err(error)) => {
            log::warn!("calendar sync failed: {error}");
            state
                .activity
                .record(format!("Google sync failed: {error}"), true);
            state.calendar.sync = SyncState::Idle;
            if matches!(error.as_str(), "Not connected to Google Calendar") {
                state.calendar.connection = Connection::SignedOut;
            }
            state.calendar.error = Some(error);
            Task::none()
        }
        CalendarMessage::CalendarTick => refresh_day(state),
        CalendarMessage::CalendarZoomed { factor, anchor } => {
            let before = state.calendar.hour_height;
            state.calendar.zoom(factor);
            hold_row_still(state, before, anchor)
        }
        CalendarMessage::CalendarZoomReset => {
            let before = state.calendar.hour_height;
            state.calendar.reset_zoom();
            hold_row_still(state, before, None)
        }
        CalendarMessage::CalendarGridScrolled(top, height) => {
            state.grid_scroll = (top, height);
            Task::none()
        }
        CalendarMessage::CalendarDayShift(days) => {
            let today =
                calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
            let origin = if state.range_view_active() {
                state.week_anchor
            } else {
                state.calendar.day
            };
            let day = origin + chrono::Duration::days(days);
            state.two_week_more = None;
            let close = if state.range_view_active() {
                state.calendar.open_event = None;
                crate::timeblocking::close(state)
            } else {
                Task::none()
            };
            state.calendar.set_day(day, today);
            if state.range_view_active() {
                state.week_anchor = day;
                state.week_scroll_x = 0.0;
            }
            state.calendar.menu_open = false;
            if state.range_view_active() {
                state.overview_month_open = false;
            } else {
                state.calendar.month_open = false;
            }
            let scroll = if state.range_view_active() {
                ui::calendar::scroll_week_to_start()
            } else {
                Task::none()
            };
            Task::batch([close, scroll, show_cached_calendar(state)])
        }
        CalendarMessage::CalendarDaySelected(day) => {
            state.two_week_more = None;
            let today =
                calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
            let close = if state.range_view_active() {
                state.calendar.open_event = None;
                crate::timeblocking::close(state)
            } else {
                Task::none()
            };
            state.calendar.set_day(day, today);
            if state.range_view_active() {
                state.week_anchor = day;
                state.week_hover = None;
                state.week_scroll_x = 0.0;
                state.overview_month_open = false;
            } else {
                state.calendar.month_open = false;
            }
            let scroll = if state.range_view_active() {
                ui::calendar::scroll_week_to_start()
            } else {
                Task::none()
            };
            Task::batch([close, scroll, show_cached_calendar(state)])
        }
        CalendarMessage::CalendarMonthToggled(MonthPicker::Panel) => {
            state.calendar.month_open = !state.calendar.month_open;
            // The picker opens on the month of the day in view, and the menu
            // gets out of its way. A range view is showing the menu itself.
            if state.calendar.month_open {
                state.calendar.month = calendar::first_of_month(state.calendar.day);
                if !state.range_view_active() {
                    state.calendar.menu_open = false;
                    state.calendar.search.open = false;
                }
            }
            Task::none()
        }
        CalendarMessage::CalendarMonthToggled(MonthPicker::Overview) => {
            state.overview_month_open = !state.overview_month_open;
            if state.overview_month_open {
                state.overview_month = calendar::first_of_month(state.week_anchor);
                state.calendar.menu_open = false;
                state.calendar.search.open = false;
            }
            Task::none()
        }
        CalendarMessage::CalendarMonthShift(picker, months) => {
            let month = match picker {
                MonthPicker::Panel => &mut state.calendar.month,
                MonthPicker::Overview => &mut state.overview_month,
            };
            let step = chrono::Months::new(months.unsigned_abs());
            let shifted = if months >= 0 {
                month.checked_add_months(step)
            } else {
                month.checked_sub_months(step)
            };
            if let Some(shifted) = shifted {
                *month = shifted;
            }
            Task::none()
        }
        CalendarMessage::CalendarMonthDayPicked(MonthPicker::Panel, day) => {
            let today =
                calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, today);
            state.calendar.month_open = false;
            if !state.range_view_active() {
                return show_cached_calendar(state);
            }
            state.panel_day_wanted = state.calendar.is_signed_in()
                && !state.calendar.has_calendar_events_on(day)
                && !state.calendar_dates().contains(&day);
            if state.panel_day_wanted {
                sync_calendar(state)
            } else {
                Task::none()
            }
        }
        CalendarMessage::CalendarMonthDayPicked(MonthPicker::Overview, day) => {
            state.two_week_more = None;
            state.calendar.open_event = None;
            let close = crate::timeblocking::close(state);
            state.week_anchor = day;
            state.week_hover = None;
            state.week_scroll_x = 0.0;
            state.overview_month_open = false;
            Task::batch([
                close,
                ui::calendar::scroll_week_to_start(),
                show_cached_calendar(state),
            ])
        }
    }
}

/// Enter Plan on Schedule; switching ranges keeps its current sidebar tab.
fn planning_sidebar(state: &mut State) -> Task<Message> {
    if state.range_view_active() {
        Task::none()
    } else {
        super::on_navigation::select_screen(state, Screen::Calendar)
    }
}

/// Scrolls the grid to make up for a zoom from `before`, so the anchored row
/// stays where it is on screen rather than sliding away as the grid resizes.
fn hold_row_still(state: &State, before: f32, anchor: Option<f32>) -> Task<Message> {
    let after = state.calendar.hour_height;
    if after == before {
        return Task::none();
    }

    let (top, height) = state.grid_scroll;
    let anchor = anchor.unwrap_or(top + height / 2.0);
    let delta = crate::ui::calendar::zoom_scroll_delta(anchor, before, after);
    crate::ui::calendar::scroll_by(state.view, delta)
}
