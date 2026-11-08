//! Windows, timers, the config file and other things the app reacts to.

use super::*;

pub(super) fn update(state: &mut State, message: SystemMessage) -> Task<Message> {
    match message {
        SystemMessage::TaskPopoverBounds(_) => Task::none(),
        SystemMessage::SystemThemeMode(mode) => {
            state.system_theme = mode;
            Task::none()
        }
        SystemMessage::EditorWindowOpened(id) => {
            if state.editor_window == Some(id) {
                if state.weekly() && state.range_view_active() {
                    ui::calendar::scroll_week_to_now(
                        state.calendar.hour_height,
                        state.config.calendar.day_end_minutes(),
                    )
                } else if state.day_plan_active() {
                    ui::calendar::scroll_to_now(
                        state.calendar.hour_height,
                        state.config.calendar.day_end_minutes(),
                    )
                } else {
                    Task::none()
                }
            } else {
                window::close(id)
            }
        }
        SystemMessage::FlashFrame(now) => {
            state.now = now;

            match &state.flash {
                Some(flash) if flash.alpha(now).is_none() => {
                    Task::done(Message::System(SystemMessage::FlashFinished))
                }
                _ => Task::none(),
            }
        }
        SystemMessage::Dismiss => dismiss(state),
        SystemMessage::SoundRendered(Ok(pcm), spec) => {
            state.audio_error = None;
            state.audio.set_gain(spec.gain);
            state.audio.play(audio::Play {
                pcm,
                repeats: spec.repeats,
                fade_in: spec.fade_in,
            });

            Task::none()
        }
        SystemMessage::SoundRendered(Err(error), _) => {
            log::warn!("could not prepare the alarm sound: {error}");
            state.audio_error = Some(error);
            Task::none()
        }
        SystemMessage::NotificationPosted(Err(error)) => {
            log::warn!("could not post the notification: {error}");
            Task::none()
        }
        SystemMessage::NotificationPosted(Ok(())) => Task::none(),
        SystemMessage::ConfigSaved(result) => {
            if let Err(error) = &result {
                log::warn!("could not save the config: {error}");
            }
            state.config_error = result.err();

            Task::none()
        }
        SystemMessage::DatabaseSyncPolled => crate::local_poll::start(state),
        SystemMessage::DatabasePolled(polled) => crate::local_poll::apply(state, polled),
        SystemMessage::OperationRetried(id) => {
            blocking::run(move || timeslay_core::calendar::operations::retry(id))
                .map(|result| Message::System(SystemMessage::OperationChanged(result)))
        }
        SystemMessage::OperationDiscarded(id) => {
            blocking::run(move || timeslay_core::calendar::operations::discard(id))
                .map(|result| Message::System(SystemMessage::OperationChanged(result)))
        }
        SystemMessage::OperationChanged(result) => {
            if let Err(error) = result {
                state.calendar.error = Some(error);
            }
            // The queue and the local copies it shows over Google's changed.
            crate::local_poll::refresh_overlay(state)
        }
        SystemMessage::ConfigReloaded(config) => {
            // Only a config that parsed gets here, so the file is usable again.
            state.config_error = None;
            if config == state.disk_config {
                return Task::none();
            }
            state.disk_config = config.clone();
            if config == state.config {
                return Task::none();
            }
            let previous = std::mem::replace(&mut state.config, config);
            if previous.views != state.config.views {
                state.view_layout_input = None;
                state.panel_border_input = None;
            }
            // An external edit wins over a save that has not started yet.
            state.save_queued = false;
            // The phone can flip this switch too, through the synced config.
            state.travel.show = state.config.travel.show;
            if previous.notepad.text != state.config.notepad.text {
                state.notepad = text_editor::Content::with_text(&state.config.notepad.text);
            }
            // Keep an open brain dump in step with an edit made outside the
            // app, without disturbing a page that is not open.
            if previous.brain_dump != state.config.brain_dump
                && let Some(date) = state.brain_dump.open
            {
                let text = state
                    .config
                    .brain_dump
                    .get(date)
                    .unwrap_or_default()
                    .to_owned();
                state.brain_dump.editor = text_editor::Content::with_text(&text);
            }
            state.audio.set_gain(state.config.alarm.sound.gain());
            state.calendar.apply_task_colors(&state.config.calendar);
            if previous.calendar.day_start_hour() != state.config.calendar.day_start_hour() {
                let day =
                    calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
                state.calendar.set_day(day, day);
                state.calendar.forget_cached_days();
            }
            let mut tasks = Vec::new();
            let pane_widths_changed =
                previous.views.calendar_width != state.config.views.calendar_width;
            let views_changed = pane_widths_changed
                || (state.sidebar.is_some() && previous.views.margin != state.config.views.margin);
            if views_changed {
                if state.sidebar.is_none() {
                    state.main_size.height = state.config.window.height;
                }
                tasks.push(apply_view_sizes(state));
            }
            if state.sidebar.is_none()
                && let Some(id) = state.main_window
            {
                if previous.window.always_on_top != state.config.window.always_on_top {
                    tasks.push(window::set_level(
                        id,
                        level(state.config.window.always_on_top),
                    ));
                }
                if !views_changed
                    && (previous.window.width != state.config.window.width
                        || previous.window.height != state.config.window.height)
                {
                    state.main_size.height = state.config.window.height;
                    tasks.push(apply_view_sizes(state));
                }
            }
            if previous.calendar != state.config.calendar {
                tasks.push(show_cached_calendar(state));
                if previous.calendar.sleep_goal_hours() != state.config.calendar.sleep_goal_hours()
                    || previous.calendar.sleep_day_start_hour()
                        != state.config.calendar.sleep_day_start_hour()
                    || previous.calendar.latest_wakeup_hour()
                        != state.config.calendar.latest_wakeup_hour()
                {
                    tasks.push(refresh_week_after_write(state));
                }
            }
            log::info!("reloaded config from disk");
            Task::batch(tasks)
        }
        SystemMessage::SaveRequested => {
            if !state.save_queued {
                return Task::none();
            }
            state.save_queued = false;
            state.disk_config = state.config.clone();
            let config = state.config.clone();

            blocking::run(move || config.save().map_err(|error| error.to_string()))
                .map(|v0| Message::System(SystemMessage::ConfigSaved(v0)))
        }
        SystemMessage::AudioStatusPolled => {
            if let audio::Status::Unavailable(reason) = state.audio.status() {
                state.audio_error = Some(reason);
            }

            Task::none()
        }
        SystemMessage::PanelCommandPolled => {
            let Some(command) = state
                .panel_command_rx
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            else {
                return Task::none();
            };

            match command {
                PanelCommand::ToggleReservation => {
                    state.config.views.panel_reserve_space =
                        !state.config.views.panel_reserve_space;
                    queue_save(state)
                }
                PanelCommand::Show => {
                    if state.main_window.is_some() {
                        return Task::none(); // already visible
                    }
                    let Some(panel) = &state.panel_config else {
                        return Task::none();
                    };
                    let settings = sidebar_settings(panel, &state.config.views);
                    let (_id, open) = window::open(settings);
                    open.map(|v0| Message::System(SystemMessage::WindowOpened(v0)))
                }
                PanelCommand::Hide => {
                    let Some(id) = state.main_window else {
                        return Task::none(); // already hidden
                    };
                    state.panel_expected_close = true;
                    window::close(id)
                }
            }
        }
        SystemMessage::InboxAddedPolled => {
            let mut added = Vec::new();
            if let Some(rx) = state.inbox_added_rx.as_ref() {
                while let Ok(event) = rx.try_recv() {
                    added.push(event);
                }
            }
            if added.is_empty() {
                return Task::none();
            }
            for event in added {
                state.timeblocking.revision += 1;
                state.calendar.events.retain(|other| other.id != event.id);
                state.calendar.replace_cached_task(&event.id, &event);
                state.calendar.add_event(event);
            }
            Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested))
        }
        SystemMessage::WindowOpened(id) => {
            if state.editor_window == Some(id) {
                return Task::none();
            }
            state.main_window = Some(id);

            // The window opens at whatever size we last saved; once the
            // monitor size is known it is widened to the default fraction of
            // the screen. The height stays as the user left it.
            let measure = window::monitor_size(id)
                .map(move |size| Message::System(SystemMessage::MonitorSized(id, size)));

            // The calendar is the front page now, so it opens on the hour the
            // needle is at rather than on midnight.
            let scroll = ui::calendar::scroll_to_now(
                state.calendar.hour_height,
                state.config.calendar.day_end_minutes(),
            );

            let sync = if state.calendar.is_signed_in() {
                auto_sync_calendar(state)
            } else {
                Task::none()
            };

            let notes = crate::notes::refresh(state);

            Task::batch([measure, sync, scroll, notes])
        }
        SystemMessage::MonitorSized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            state.monitor_size = size;
            if state.task_creation_window {
                return Task::none();
            }
            let Some(size) = size else {
                return Task::none();
            };

            // The panel is pinned to its configured width and the full height
            // of the monitor, and never widens to the default fraction.
            if let Some(sidebar) = &state.sidebar {
                let width =
                    state.config.views.calendar_width(sidebar.width) + state.screen_margin_width();
                log::debug!("panel filling {}x{}", width, size.height);
                return window::resize(id, Size::new(width, size.height));
            }

            // Only a window that has never been sized gets the default
            // fraction. A width the user chose, or the one saved with the
            // editor open, must survive a relaunch.
            if state.config.window.width != Config::default().window.width
                || state.config.views.calendar_width.is_some()
            {
                return Task::none();
            }

            let width = launch_width(size.width);
            let height = state.config.window.height;
            state.config.window.width = width;

            Task::batch([
                window::resize(id, Size::new(width, height)),
                queue_save(state),
            ])
        }
        SystemMessage::WeekMonitorSized(id, size) => {
            if state.main_window != Some(id) || !state.range_view_active() {
                return Task::none();
            }
            state.monitor_size = size;
            Task::none()
        }
        SystemMessage::WindowResized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            state.main_size = size;
            if state.task_creation_window {
                return Task::none();
            }
            log::debug!("window is now {}x{}", size.width, size.height);

            // The panel's geometry is not the user's saved window, so it must
            // not be written back over it.
            if state.sidebar.is_some() {
                return Task::none();
            }

            state.config.window.width = size.width;
            state.config.window.height = size.height;

            queue_save(state)
        }
        SystemMessage::WindowClosed(id) => {
            if state.editor_window == Some(id) {
                state.editor_window = None;
                // Overview visibility belongs to its toggle, even if the
                // compositor closes the floating surface. Reconciliation
                // recreates it while the panel is visible.
                if state.range_view_active() || state.day_plan_active() {
                    return Task::none();
                }
                state.calendar.open_event = None;
                state.calendar.edit_draft = None;
                state.calendar.detail_event = None;
                return crate::timeblocking::close(state);
            }
            if state.main_window == Some(id) {
                if state.panel_expected_close {
                    state.panel_expected_close = false;
                    state.main_window = None;
                    return Task::none();
                }
                // Closing the clock quits, even though a daemon would
                // otherwise keep running with no windows left.
                if !state.config.task_work.is_empty() {
                    let config = state.config.clone();
                    return blocking::run(move || config.save()).then(|_| iced::exit());
                }
                return iced::exit();
            }

            if state.flash.as_ref().is_some_and(|flash| flash.window == id) {
                state.flash = None;
            }

            Task::none()
        }
        SystemMessage::FlashFinished => {
            let Some(flash) = state.flash.take() else {
                return Task::none();
            };

            window::close(flash.window)
        }
        SystemMessage::KeyPressed(key, modifiers) => on_key(state, key, modifiers),
        SystemMessage::OpenGoogleCalendar => {
            if let Err(error) = calendar::oauth::open_in_browser(calendar::WEB_URL) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }
        SystemMessage::DockRequested => {
            if let Err(error) = crate::dock::launch::open_dock() {
                state.calendar.error = Some(format!("Could not open the dock: {error:#}"));
                return Task::none();
            }

            state.calendar.menu_open = false;
            state.calendar.search.open = false;
            let Some(id) = state.main_window else {
                return Task::none();
            };
            // The panel process stays up with its window hidden, so it can be
            // brought back by the dock.
            state.panel_expected_close = state.sidebar.is_some();
            state.panel_visible.store(false, Ordering::Relaxed);
            window::close(id)
        }
    }
}
