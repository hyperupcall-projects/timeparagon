//! Changes made on the Settings page.

use super::*;
use chrono::Datelike;

pub(super) fn update(state: &mut State, message: SettingsMessage) -> Task<Message> {
    match message {
        SettingsMessage::NotesDirectoryChanged(value) => {
            state.notes.directory_input = value;
            Task::none()
        }
        SettingsMessage::NotesDirectoryBrowse => blocking::run(|| {
            rfd::FileDialog::new()
                .set_title("Choose notes folder")
                .pick_folder()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .map(|path| Message::Settings(SettingsMessage::NotesDirectoryPicked(path))),
        SettingsMessage::NotesDirectoryPicked(path) => {
            if let Some(path) = path {
                state.notes.directory_input = path;
                return update(state, SettingsMessage::NotesDirectoryApply);
            }
            Task::none()
        }
        SettingsMessage::NotesDirectoryApply => {
            let settings = timeslay_core::config::NotesSettings {
                directory: state.notes.directory_input.trim().to_owned(),
            };
            match state.notes.configure(settings.path()) {
                Ok(()) => {
                    state.config.notes = settings;
                    queue_save(state)
                }
                Err(error) => {
                    state.notes.error = Some(format!("Could not change notes folder: {error}"));
                    Task::none()
                }
            }
        }
        SettingsMessage::DatabaseSyncUrlChanged(url) => {
            state.config.database_sync.url = url;
            queue_save(state)
        }
        SettingsMessage::ShortcutInputChanged(action, value) => {
            let editor = &mut state.shortcut_editors[action.index()];
            editor.input = value;
            editor.result = None;
            Task::none()
        }
        SettingsMessage::ShortcutAdded(action) => {
            let editor = &mut state.shortcut_editors[action.index()];
            let spec = editor.input.trim().to_owned();
            if editor.busy || spec.is_empty() {
                return Task::none();
            }
            let mut shortcuts = state.config.panel.shortcuts(action).to_vec();
            let problem = if let Err(error) = panel::validate(&spec) {
                Some(format!("{error:#}"))
            } else if shortcuts
                .iter()
                .any(|known| panel::same_shortcut(known, &spec))
            {
                Some(format!("{spec} is already a {} shortcut", action.label()))
            } else {
                None
            };
            if let Some(problem) = problem {
                editor.result = Some(Err(problem));
                return Task::none();
            }
            shortcuts.push(spec);
            apply_shortcuts(state, action, shortcuts)
        }
        SettingsMessage::ShortcutRemoved(action, index) => {
            let mut shortcuts = state.config.panel.shortcuts(action).to_vec();
            if state.shortcut_editors[action.index()].busy || index >= shortcuts.len() {
                return Task::none();
            }
            shortcuts.remove(index);
            apply_shortcuts(state, action, shortcuts)
        }
        SettingsMessage::ShortcutsApplied(action, result) => {
            let editor = &mut state.shortcut_editors[action.index()];
            editor.busy = false;
            match result {
                Ok(shortcuts) => {
                    editor.input.clear();
                    editor.result = Some(Ok(if shortcuts.is_empty() {
                        format!("{} has no shortcut", action.label())
                    } else {
                        format!("{} is {}", action.label(), shortcuts.join(", "))
                    }));
                    state.config.panel.set_shortcuts(action, shortcuts);
                    queue_save(state)
                }
                Err(error) => {
                    editor.result = Some(Err(error));
                    Task::none()
                }
            }
        }
        SettingsMessage::ViewLayoutInputChanged(field, input) => {
            if !input.chars().all(|c| c.is_ascii_digit()) {
                return Task::none();
            }
            let (min, max) = field.bounds();
            if let Ok(value) = input.parse::<f32>()
                && (min..=max).contains(&value)
            {
                super::update(state, field.changed(value))
            } else {
                state.view_layout_input = Some((field, input));
                Task::none()
            }
        }
        SettingsMessage::ViewLayoutInputSubmitted(field) => {
            let value = state
                .view_layout_input
                .take()
                .filter(|(edited, _)| *edited == field)
                .and_then(|(_, input)| input.parse::<f32>().ok())
                .filter(|value| value.is_finite())
                .unwrap_or_else(|| field.value(state));
            super::update(state, field.changed(value))
        }
        SettingsMessage::CalendarWeekWidthChanged(width)
        | SettingsMessage::CalendarMonthWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            let width = Some(width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            ));
            if matches!(message, SettingsMessage::CalendarWeekWidthChanged(_)) {
                state.config.views.calendar_week_width = width;
            } else {
                state.config.views.calendar_month_width = width;
            }
            queue_save(state)
        }
        SettingsMessage::CalendarWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.calendar_width = Some(width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            ));
            let resize = apply_view_sizes(state);
            Task::batch([resize, queue_save(state)])
        }
        SettingsMessage::TimeBlockWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.time_block_width = width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            );
            queue_save(state)
        }
        SettingsMessage::DetailWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.detail_width = width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            );
            queue_save(state)
        }
        SettingsMessage::ScreenMarginChanged(margin) => {
            state.view_layout_input = None;
            if !margin.is_finite() {
                return Task::none();
            }
            state.config.views.margin = margin.clamp(0.0, config::ViewSettings::MAX_MARGIN);
            let resize = if state.sidebar.is_some() {
                apply_view_sizes(state)
            } else {
                Task::none()
            };
            Task::batch([resize, queue_save(state)])
        }
        SettingsMessage::DockBorderRadiusChanged(radius) => {
            state.config.views.dock_border_radius =
                radius.clamp(0.0, config::ViewSettings::MAX_BORDER_RADIUS);
            queue_save(state)
        }
        SettingsMessage::PanelBorderRadiusChanged(radius) => {
            state.config.views.panel_border_radius =
                Some(radius.clamp(0.0, config::ViewSettings::MAX_BORDER_RADIUS));
            queue_save(state)
        }
        SettingsMessage::DockReserveSpaceToggled(enabled) => {
            state.config.views.dock_reserve_space = enabled;
            queue_save(state)
        }
        SettingsMessage::PanelReserveSpaceToggled(enabled) => {
            state.config.views.panel_reserve_space = enabled;
            queue_save(state)
        }
        SettingsMessage::DockWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.dock_width = width.clamp(
                config::ViewSettings::MIN_DOCK_WIDTH,
                config::ViewSettings::MAX_DOCK_WIDTH,
            );
            queue_save(state)
        }
        SettingsMessage::DockLeftMarginChanged(margin)
        | SettingsMessage::DockRightMarginChanged(margin) => {
            let left = matches!(message, SettingsMessage::DockLeftMarginChanged(_));
            state.view_layout_input = None;
            if !margin.is_finite() {
                return Task::none();
            }
            let margin = Some(margin.clamp(0.0, config::ViewSettings::MAX_DOCK_MARGIN));
            if left {
                state.config.views.dock_left_margin = margin;
            } else {
                state.config.views.dock_right_margin = margin;
            }
            queue_save(state)
        }
        SettingsMessage::DockSideSelected(side) => {
            state.config.views.dock_side = side;
            queue_save(state)
        }
        SettingsMessage::DockBorderOuterWidthChanged(width) => super::update(
            state,
            Message::Settings(SettingsMessage::PanelBorderOuterWidthChanged(width)),
        ),
        SettingsMessage::DockBorderInnerWidthChanged(width) => super::update(
            state,
            Message::Settings(SettingsMessage::PanelBorderInnerWidthChanged(width)),
        ),
        SettingsMessage::DockBorderColorInput(layer, input) => super::update(
            state,
            Message::Settings(SettingsMessage::PanelBorderColorInput(layer, input)),
        ),
        SettingsMessage::DockBorderColorSubmitted(layer) => super::update(
            state,
            Message::Settings(SettingsMessage::PanelBorderColorSubmitted(layer)),
        ),
        SettingsMessage::PanelBorderOuterWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.panel_border_outer_width =
                width.clamp(0.0, config::ViewSettings::MAX_BORDER_WIDTH);
            state.config.views.dock_border_outer_width =
                state.config.views.panel_border_outer_width;
            queue_save(state)
        }
        SettingsMessage::PanelBorderInnerWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.panel_border_inner_width =
                width.clamp(0.0, config::ViewSettings::MAX_BORDER_WIDTH);
            state.config.views.dock_border_inner_width =
                state.config.views.panel_border_inner_width;
            queue_save(state)
        }
        SettingsMessage::PanelBorderColorInput(layer, input) => {
            let digits = input.trim().trim_start_matches('#');
            if digits.len() > 8 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
                return Task::none();
            }
            state.panel_border_input = Some((layer, input.clone()));
            let Ok(color) = crate::hex::parse(&input) else {
                return Task::none();
            };
            match layer {
                BorderLayer::Outer => state.config.views.panel_border_outer_color = color.into(),
                BorderLayer::Inner => state.config.views.panel_border_inner_color = color.into(),
            }
            state.config.views.dock_border_outer_color =
                state.config.views.panel_border_outer_color;
            state.config.views.dock_border_inner_color =
                state.config.views.panel_border_inner_color;
            queue_save(state)
        }
        SettingsMessage::PanelBorderColorSubmitted(_) => {
            state.panel_border_input = None;
            Task::none()
        }
        SettingsMessage::FadeDurationChanged(duration) => {
            state.view_layout_input = None;
            if !duration.is_finite() {
                return Task::none();
            }
            state.config.views.fade_duration_ms = duration
                .round()
                .clamp(0.0, config::ViewSettings::MAX_FADE_DURATION_MS as f32)
                as u32;
            crate::window_effect::prepare(&state.config.views);
            queue_save(state)
        }
        SettingsMessage::WindowAnimationSelected(animation) => {
            state.config.views.window_animation = animation;
            crate::window_effect::prepare(&state.config.views);
            queue_save(state)
        }
        SettingsMessage::RestartRequested => {
            let config = state.config.clone();
            blocking::run(move || {
                let saved = config.save();
                (saved, crate::dock::launch::relaunch())
            })
            .then(|(saved, relaunched)| {
                if let Err(error) = &saved {
                    log::error!("could not save before restarting: {error:#}");
                }
                match relaunched {
                    Ok(()) => iced::exit(),
                    Err(error) => {
                        log::error!("could not restart: {error:#}");
                        Task::none()
                    }
                }
            })
        }
        SettingsMessage::DockHourLinesToggled(on) => {
            state.config.views.dock_hour_lines = on;
            queue_save(state)
        }
        SettingsMessage::BorderRadiusChanged(radius) => {
            state.view_layout_input = None;
            if !radius.is_finite() {
                return Task::none();
            }
            state.config.views.border_radius =
                radius.clamp(0.0, config::ViewSettings::MAX_BORDER_RADIUS);
            queue_save(state)
        }
        SettingsMessage::ThemeSelected(name) => {
            state.config.appearance.theme = name;
            queue_save(state)
        }
        SettingsMessage::DistanceUnitSelected(unit) => {
            state.config.appearance.distance_unit = unit;
            queue_save(state)
        }
        SettingsMessage::TimeFormatSelected(format) => {
            state.config.appearance.time_format = format;
            queue_save(state)
        }
        SettingsMessage::FirstDayOfWeekSelected(first_day) => {
            state.config.calendar.first_day_of_week = first_day;
            // The month and two-week views now span different days.
            state.two_week_more = None;
            Task::batch([queue_save(state), show_cached_calendar(state)])
        }
        SettingsMessage::AlwaysOnTopToggled(on_top) => {
            state.config.window.always_on_top = on_top;

            let apply = state
                .main_window
                .map(|id| window::set_level(id, level(on_top)))
                .unwrap_or(Task::none());

            Task::batch([apply, queue_save(state)])
        }
        SettingsMessage::ShowTasksToggled(show) => {
            state.config.calendar.show_tasks = show;
            queue_save(state)
        }
        SettingsMessage::SourceColorEditorToggled(id) => {
            state.calendar.color_editor =
                (state.calendar.color_editor.as_deref() != Some(id.as_str())).then_some(id);
            Task::none()
        }
        SettingsMessage::TaskColorPicked(id, color) => {
            state.config.calendar.set_task_color(&id, color);
            state.calendar.apply_task_colors(&state.config.calendar);
            state.calendar.color_editor = None;
            queue_save(state)
        }
        SettingsMessage::CalendarColorPicked(id, color) => {
            match color {
                Some(color) => {
                    state
                        .config
                        .calendar
                        .calendar_colors
                        .insert(id, crate::hex::HexColor(color));
                }
                None => {
                    state.config.calendar.calendar_colors.remove(&id);
                }
            }
            state.calendar.color_editor = None;
            queue_save(state)
        }
        SettingsMessage::TimeSlayColorPicked(id, color) => {
            if let Some(source) = calendar::sources::TimeSlaySource::from_id(&id) {
                source.set_color(&mut state.config.calendar, color);
                state.calendar.apply_task_colors(&state.config.calendar);
                state.calendar.color_editor = None;
            }
            queue_save(state)
        }
        SettingsMessage::GoogleClientIdChanged(id) => {
            state.config.calendar.client_id = id;
            queue_save(state)
        }
        SettingsMessage::GoogleClientSecretChanged(secret) => {
            state.config.calendar.client_secret = secret;
            queue_save(state)
        }
        SettingsMessage::GoogleConnectRequested => {
            if state.calendar.is_busy() {
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.sync = SyncState::Connecting;
            state.calendar.error = None;

            blocking::run(move || calendar::connect(client_id, client_secret))
                .map(|v0| Message::Settings(SettingsMessage::GoogleConnected(v0)))
        }
        SettingsMessage::GoogleConnected(Ok(_tokens)) => {
            state.calendar.connection = Connection::SignedIn;
            state.calendar.sync = SyncState::Idle;
            state.calendar.error = None;
            let people = if state.screen == Screen::People {
                crate::people::refresh(state)
            } else {
                Task::none()
            };
            Task::batch([sync_calendar(state), people])
        }
        SettingsMessage::GoogleConnected(Err(error)) => {
            log::warn!("Google sign-in failed: {error}");
            state.calendar.sync = SyncState::Idle;
            state.calendar.connection = Connection::SignedOut;
            state.calendar.error = Some(error);
            Task::none()
        }
        SettingsMessage::GoogleDisconnectRequested => {
            calendar::disconnect();
            state.calendar = calendar::Store::new(state.config.calendar.day_start_hour());
            // The reset takes the store with it, so the directory is put
            // straight back: days keep being written across a disconnect.
            state.calendar.apply_task_colors(&state.config.calendar);
            state.calendar.cache_days_in(crate::task_db::path());
            state.calendar.connection = Connection::SignedOut;
            crate::people::forget_primary(state)
        }
        SettingsMessage::CalendarSyncMinutesChanged(minutes) => {
            state.config.calendar.sync_minutes = minutes.clamp(
                config::CalendarSettings::MIN_SYNC_MINUTES,
                config::CalendarSettings::MAX_SYNC_MINUTES,
            );
            queue_save(state)
        }
        SettingsMessage::SleepTimeChanged(day, bedtime, value) => {
            let Ok(time) = chrono::NaiveTime::parse_from_str(&value, "%H:%M") else {
                return Task::none();
            };
            let today = crate::clock::now().date_naive();
            let from = today + chrono::Duration::days(i64::from(day.days_since(today.weekday())));
            let mut times =
                state.config.calendar.sleep_schedule[day.num_days_from_monday() as usize];
            if bedtime {
                times.bedtime = time;
            } else {
                times.wakeup = time;
            }
            calendar::sleep::set_weekday_times(&mut state.config.calendar, from, times);
            state.calendar.invalidate_other_days();
            Task::batch([queue_save(state), refresh_week_after_write(state)])
        }
        SettingsMessage::SleepEstimatedStart(value) => {
            if let Some(record) = state
                .calendar
                .edit_draft
                .as_mut()
                .and_then(|d| d.sleep.as_mut())
            {
                record.estimated_start = value;
            }
            Task::none()
        }
        SettingsMessage::SleepEstimatedEnd(value) => {
            if let Some(record) = state
                .calendar
                .edit_draft
                .as_mut()
                .and_then(|d| d.sleep.as_mut())
            {
                record.estimated_end = value;
            }
            Task::none()
        }
        SettingsMessage::SleepSaveRequested => {
            let Some(draft) = state.calendar.edit_draft.as_ref() else {
                return Task::none();
            };
            let Some(mut record) = draft.sleep.clone() else {
                return Task::none();
            };
            if draft.saving {
                return Task::none();
            }
            if calendar::parse_clock(&draft.start_text).is_none()
                || calendar::parse_clock(&draft.end_text).is_none()
                || calendar::deadline::parse_date(&draft.date_text).is_err()
                || calendar::deadline::parse_date(&draft.end_date_text).is_err()
                || draft.end <= draft.start
            {
                state.calendar.error =
                    Some("Enter valid dates and times, with wake-up after sleep start".into());
                return Task::none();
            }
            let Some(mut event) = state.calendar.event(&draft.id).cloned() else {
                return Task::none();
            };
            if event.id != calendar::sleep::NEW_ID
                && (event.start != draft.start || event.end != draft.end)
            {
                return super::update(
                    state,
                    Message::Event(EventMessage::EventTimeChanged {
                        id: event.id.clone(),
                        start: draft.start,
                        end: draft.end,
                    }),
                );
            }
            let new = event.id == calendar::sleep::NEW_ID;
            record.automatic = false;
            if new {
                record.wake_date = draft.end.date_naive();
            }
            event.start = draft.start;
            event.end = draft.end;
            event.details.sleep = Some(record);
            state.calendar.edit_draft.as_mut().unwrap().saving = true;
            state.timeblocking.revision += 1;
            state.calendar.error = None;
            let settings = state.config.calendar.clone();
            blocking::run(move || calendar::sleep::save(&settings, event, new))
                .map(|v0| Message::Settings(SettingsMessage::SleepSaved(v0)))
        }
        SettingsMessage::SleepSaved(Ok(event)) => {
            let id = event.id.clone();
            state.calendar.forget_event_everywhere(&id);
            state.calendar.detail_event = None;
            state.calendar.edit_draft = Some(calendar::EditDraft::from_event(&event));
            state.calendar.add_event(event);
            state.calendar.open_event = Some(id);
            refresh_week_after_write(state)
        }
        SettingsMessage::SleepSaved(Err(error)) => {
            if let Some(draft) = &mut state.calendar.edit_draft {
                draft.saving = false;
            }
            state.calendar.error = Some(error);
            Task::none()
        }
        SettingsMessage::CalendarDayEndChanged(hour) => {
            state.config.calendar.day_end_hour =
                hour.min(config::CalendarSettings::MAX_DAY_END_HOUR);
            // The day's window moved, so the day in view and the events that
            // belong to it may have changed too. This also puts the view back
            // on today if the user had navigated away.
            let day = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, day);
            state.week_anchor = day;
            // The day's window moved, so the cached days no longer describe
            // the days they are filed under.
            state.calendar.forget_cached_days();
            let save = queue_save(state);
            Task::batch([save, sync_calendar(state)])
        }
        SettingsMessage::CalendarWeekRangeChanged(range) => {
            state.config.calendar.set_week_range(range);
            Task::batch([queue_save(state), show_cached_calendar(state)])
        }
        SettingsMessage::DefaultCalendarSelected(id) => {
            state.config.calendar.default_calendar = if id.is_empty() { None } else { Some(id) };
            queue_save(state)
        }
    }
}
