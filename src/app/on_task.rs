//! Tasks: dragging, completing and deleting them.

use super::*;

pub(super) fn update(state: &mut State, message: TaskMessage) -> Task<Message> {
    match message {
        TaskMessage::WeekDragScroll => ui::calendar::scroll_week_drag(state),
        TaskMessage::TaskDragMoved(window, position) => {
            if state.timeblocking.dragging.is_none() {
                return Task::none();
            }
            state.timeblocking.drag_window = Some(window);
            state.timeblocking.drag_pointer = Some(position);
            ui::task_drop::probe_drag(state)
        }
        TaskMessage::TaskDragHovered(id, position, message) => {
            // Moves queue up probes; only the answer for where the pointer is
            // now still describes the drop.
            let current = state.timeblocking.drag_pointer.and_then(|pointer| {
                ui::task_drop::range_position(
                    state,
                    state.timeblocking.drag_window.or(state.main_window)?,
                    pointer,
                )
            });
            if state.timeblocking.dragging.as_deref() == Some(&id) && current == Some(position) {
                state.timeblocking.drop_hover =
                    message.as_deref().and_then(ui::task_drop::hover_of);
            }
            Task::none()
        }
        TaskMessage::TaskDragReleased(window) => {
            if let Some(id) = state.timeblocking.dragging.clone()
                && state.main_window == Some(window)
                && let Some(position) = state.timeblocking.drag_pointer
                && let Some(position) = ui::task_drop::range_position(state, window, position)
            {
                return ui::task_drop::release(id, position);
            }
            crate::timeblocking::update(state, crate::timeblocking::Action::CancelDrag)
        }
        TaskMessage::TaskDragDropped(id, message) => {
            if state.timeblocking.dragging.as_deref() != Some(&id) {
                return Task::none();
            }
            dispatch(
                state,
                message.map_or(
                    Message::TimeBlocking(crate::timeblocking::Action::CancelDrag),
                    |message| *message,
                ),
            )
        }
        TaskMessage::TaskDeleteRequested(id) => {
            if state
                .calendar
                .event(&id)
                .is_some_and(|e| e.is_calendar_task())
            {
                return super::on_event::update(state, EventMessage::EventDeleteRequested(id));
            }

            // Only Google Tasks can be deleted this way; calendar events use
            // EventDeleteRequested instead.
            let is_task = state
                .calendar
                .event(&id)
                .map(|event| event.is_task())
                .unwrap_or(false);
            if !is_task {
                return Task::none();
            }
            state.calendar.pending_delete = Some(id);
            Task::none()
        }
        TaskMessage::TaskDeleteConfirmed(id) => {
            if state.calendar.pending_delete.as_deref() != Some(id.as_str()) {
                return Task::none();
            }
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.revision += 1;
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|event| event.is_task())
                .cloned()
            else {
                state.calendar.pending_delete = None;
                return Task::none();
            };

            let app_calendar = state.config.calendar.app_calendar.clone();
            state.calendar.error = None;
            state.timeblocking.busy.insert(id.clone());

            blocking::run(move || calendar::delete_task(&task, app_calendar.as_deref()))
                .map(move |result| Message::Task(TaskMessage::TaskDeleted(id.clone(), result)))
        }
        TaskMessage::TaskDeleted(id, Ok(())) => {
            state.timeblocking.busy.remove_task(&id);
            state.timeblocking.revision += 1;
            state.calendar.pending_delete = None;
            if let Some(task) = state.calendar.event(&id) {
                state
                    .config
                    .task_work
                    .stop(&timeblocking::task_key(task), crate::clock::utc_now());
            }
            // The task is gone on Google's side. Drop every chip that was
            // showing it, including a stale copy on the list it was moved
            // from and the calendar block that held its time.
            state.calendar.forget_task(&id);
            let save = queue_save(state);
            Task::batch([save, refresh_week_after_write(state)])
        }
        TaskMessage::TaskDeleted(id, Err(error)) => {
            state.timeblocking.busy.remove_task(&id);
            state.timeblocking.revision += 1;
            state.calendar.pending_delete = None;
            log::warn!("could not delete the task: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }
        TaskMessage::TaskToggled(_) | TaskMessage::TaskIgnoreToggled(_) => {
            let ignoring = matches!(&message, TaskMessage::TaskIgnoreToggled(_));
            let (TaskMessage::TaskToggled(id) | TaskMessage::TaskIgnoreToggled(id)) = message
            else {
                unreachable!()
            };
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.revision += 1;
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|event| event.is_task())
                .cloned()
            else {
                return Task::none();
            };
            let previous = timeblocking::task_status(&task);
            let status = if ignoring {
                if previous == timeblocking::TaskStatus::Ignored {
                    timeblocking::TaskStatus::Active
                } else {
                    timeblocking::TaskStatus::Ignored
                }
            } else if previous == timeblocking::TaskStatus::Completed {
                timeblocking::TaskStatus::Active
            } else {
                timeblocking::TaskStatus::Completed
            };

            // Update the chip straight away, then put it back if Google
            // refuses the write.
            state.timeblocking.busy.insert(id.clone());
            let completed = status != timeblocking::TaskStatus::Active;
            if completed && let Some(task) = state.calendar.event(&id) {
                state.config.task_work.stop(
                    &crate::timeblocking::task_key(task),
                    crate::clock::utc_now(),
                );
                state.timeblocking.work_now = Some(crate::clock::utc_now());
                let _ = queue_save(state);
            }
            if let Some(event) = state.calendar.event_mut(&id) {
                timeblocking::apply_status(event, status);
            }

            state.calendar.error = None;

            blocking::run(move || calendar::set_linked_task_status(&task, status)).map(
                move |result| {
                    Message::Task(TaskMessage::TaskCompletionSaved(
                        id.clone(),
                        previous,
                        result,
                    ))
                },
            )
        }
        TaskMessage::TaskCompletionSaved(id, previous, Ok(task)) => {
            if let Some(event) = state.calendar.event_mut(&id) {
                *event = task.clone();
            }
            state.timeblocking.busy.remove_task(&id);
            state.timeblocking.revision += 1;
            state.calendar.replace_cached_task(&id, &task);
            let record = if (previous == timeblocking::TaskStatus::Completed)
                != (timeblocking::task_status(&task) == timeblocking::TaskStatus::Completed)
            {
                crate::habits::record_occurrence(state, &task)
            } else {
                Task::none()
            };
            Task::batch([
                record,
                Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested)),
            ])
        }
        TaskMessage::TaskCompletionSaved(id, previous, Err(error)) => {
            state.timeblocking.busy.remove_task(&id);
            state.timeblocking.revision += 1;
            if let Some(event) = state.calendar.event_mut(&id) {
                timeblocking::apply_status(event, previous);
            }

            log::warn!("could not update the task: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }
    }
}
