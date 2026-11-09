//! The panel that edits an event that already exists.

use super::*;
use iced::widget::column;

/// The in-place editor for a calendar event or time block.
///
/// It is the same set of fields the new-event dialog collects, bound to the
/// event's live [`calendar::EditDraft`] so changes appear in the pane rather
/// than in a popup. Save writes them to Google; Close discards the draft.
pub fn event_editor<'a>(
    state: &'a State,
    event: &'a Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let Some(draft) = state
        .calendar
        .edit_draft
        .as_ref()
        .filter(|draft| draft.id == event.id)
    else {
        return column![
            titled(
                state,
                event,
                super::super::links::text(event.title.clone(), metrics.label() + 2.0),
                metrics,
            ),
            text(schedule_line(event, state.config.appearance.time_format))
                .size(metrics.label())
                .style(text::secondary),
        ]
        .spacing(metrics.spacing)
        .into();
    };

    if draft.loading {
        let mut loading = column![text("Loading recurrence…").size(metrics.label())].spacing(6);
        if let Some(error) = &state.calendar.error {
            loading = loading.push(text(error).style(text::danger)).push(
                crate::ui::controls::quiet_button("Retry")
                    .on_press(Message::Event(EventMessage::EventOpened(event.id.clone()))),
            );
        }
        return loading.into();
    }
    let label = |name: &'static str| text(name).size(metrics.label()).style(text::secondary);

    let title: Element<'a, Message> = if draft.editing_title {
        column![
            crate::ui::controls::text_input("Title", &draft.title)
                .id(iced::widget::Id::new("event-title"))
                .on_input(|v0| Message::EventEdit(EventEditMessage::EventEditTitleChanged(v0)))
                .on_submit(Message::EventEdit(EventEditMessage::EventEditSaveRequested))
                .size(metrics.label() + 6.0)
                .width(Fill),
            text("Press Enter to save")
                .size(metrics.label() - 2.0)
                .style(text::secondary),
        ]
        .spacing(4)
        .into()
    } else {
        button(text(&draft.title).size(metrics.label() + 6.0).font(Font {
            weight: Weight::Bold,
            ..Font::DEFAULT
        }))
        .padding(0)
        .width(Fill)
        .style(crate::ui::controls::ghost)
        .on_press(Message::Event(EventMessage::EventTitleEdit))
        .into()
    };
    // Habits and time blocks are always timed, so only a plain event can be
    // switched to whole days.
    let can_be_all_day =
        !event.details.habit && !crate::timeblocking::is_block(event, &state.config.calendar);
    let mut start = column![label(if event.details.habit {
        "Start day"
    } else if draft.all_day {
        "Start Date"
    } else {
        "Start Time"
    })]
    .spacing(6)
    .width(Fill);
    if can_be_all_day {
        start = start.push(
            crate::ui::controls::checkbox(draft.all_day)
                .label("All day")
                .text_size(metrics.label())
                .on_toggle(|v0| Message::EventEdit(EventEditMessage::EventEditAllDayToggled(v0))),
        );
    }
    start = start.push(super::super::controls::date(
        state.config.appearance.time_format,
        &draft.date_text,
        false,
        false,
        true,
        metrics.label(),
        |v0| Message::EventEdit(EventEditMessage::EventEditDateChanged(v0)),
    ));
    if !draft.all_day {
        start = start.push(super::super::controls::time(
            state.config.appearance.time_format,
            &draft.start_text,
            true,
            metrics.label(),
            |v0| Message::EventEdit(EventEditMessage::EventEditStartChanged(v0)),
        ));
    }
    let targets = state.calendar.move_targets(event, &state.config.calendar);
    let source: Option<Element<'a, Message>> = if targets.is_empty() || event.is_calendar_task() {
        None
    } else {
        let options: Vec<CalendarChoice> = targets
            .into_iter()
            .map(|calendar| CalendarChoice {
                id: calendar.id.clone(),
                name: calendar.name.clone(),
                color: calendar.color,
            })
            .collect();
        let selected = options
            .iter()
            .find(|option| option.id == draft.calendar_id)
            .cloned();
        Some(calendar_select(
            options,
            selected,
            draft.calendar_dropdown_open,
            metrics,
            |id| Message::EventEdit(EventEditMessage::EventEditCalendarSelected(id)),
            Message::EventEdit(EventEditMessage::EventEditCalendarDropdownToggled),
        ))
    };
    let mut body = column![
        titled(state, event, title, metrics),
        source,
        super::super::description::preview(&event.id, &draft.description, metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill);
    for meeting in super::super::meetings::links(event) {
        body = body.push(super::super::meetings::join_button(meeting, metrics));
    }
    body = body
        .push(event_projects(state, &draft.projects, metrics, |v0| {
            Message::EventEdit(EventEditMessage::EventEditProjects(v0))
        }))
        .push(start);
    if event.details.habit {
        body = body.push(event_habit_picker(state, event, metrics));
        body = body.push(super::super::recurrence::editor(
            &draft.rule,
            draft.start.date_naive(),
            state.config.calendar.first_day_of_week,
            metrics,
            false,
            |v0| Message::EventEdit(EventEditMessage::EventEditRecurrence(v0)),
        ));
    } else {
        // An all-day end date is the last day covered, not Google's
        // exclusive end.
        let mut end = column![
            label(if draft.all_day {
                "End Date"
            } else {
                "End Time"
            }),
            super::super::controls::date(
                state.config.appearance.time_format,
                &draft.end_date_text,
                false,
                false,
                true,
                metrics.label(),
                |v0| Message::Event(EventMessage::EventEndDateChanged(v0))
            ),
        ]
        .spacing(6)
        .width(Fill);
        if !draft.all_day {
            end = end.push(super::super::controls::time(
                state.config.appearance.time_format,
                &draft.end_text,
                true,
                metrics.label(),
                |v0| Message::EventEdit(EventEditMessage::EventEditEndChanged(v0)),
            ));
        }
        body = body.push(end);
        let mut labels = column![label("Labels")].spacing(6).width(Fill);
        for value in &draft.labels {
            labels = labels.push(
                row![
                    text(value).size(metrics.label()).width(Fill),
                    button("×")
                        .on_press(Message::Event(EventMessage::EventLabelRemove(
                            value.clone()
                        )))
                        .style(crate::ui::controls::ghost)
                ]
                .spacing(6),
            );
        }
        labels = labels.push(
            row![
                crate::ui::controls::text_input("Add a label…", &draft.label_draft)
                    .on_input(|v0| Message::Event(EventMessage::EventLabelDraft(v0)))
                    .on_submit(Message::Event(EventMessage::EventLabelAdd))
                    .size(metrics.label()),
                crate::ui::controls::action_button("Add")
                    .on_press(Message::Event(EventMessage::EventLabelAdd)),
            ]
            .spacing(8),
        );
        body = body.push(labels);
    }

    if crate::timeblocking::is_block(event, &state.config.calendar) {
        body = body.push(super::super::timeblocking::propagation_controls(
            state, event, metrics,
        ));
        body = body.push(super::super::recurrence::editor(
            &draft.rule,
            draft.start.date_naive(),
            state.config.calendar.first_day_of_week,
            metrics,
            false,
            |v| Message::EventEdit(EventEditMessage::EventEditRecurrence(v)),
        ));
    }
    if let Some(error) = &state.calendar.error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }

    if state.calendar.open_event.as_deref() != Some(event.id.as_str()) {
        if let Some(draft) = &state.calendar.edit_draft {
            body = body.push(event_options(&draft.options, metrics, |v0| {
                Message::EventEdit(EventEditMessage::EventEditOptions(v0))
            }));
        }
        body = body.push(event_save_control(state, event, metrics));
    }
    body.into()
}

/// Google-style notification rows followed by availability and visibility.
pub(super) fn event_options<'a>(
    options: &'a calendar::event_options::EventOptions,
    metrics: &Metrics,
    message: fn(calendar::event_options::Action) -> Message,
) -> Element<'a, Message> {
    use calendar::event_options::{Action, Availability, Method, Unit, Visibility};
    let size = metrics.label();
    let mut notifications = column![
        text("Notifications").size(size).style(text::secondary),
        crate::ui::controls::checkbox(options.reminders.use_default)
            .label("Use calendar default notifications")
            .on_toggle(move |v| message(Action::Defaults(v)))
            .text_size(size),
    ]
    .spacing(8)
    .width(Fill);
    if options.reminders.use_default {
        for reminder in &options.default_reminders {
            notifications = notifications.push(
                text(format!(
                    "{} · {} minutes before",
                    reminder.method, reminder.minutes
                ))
                .size(size)
                .style(text::secondary),
            );
        }
    }
    if !options.reminders.use_default {
        if options.reminders.overrides.is_empty() {
            notifications =
                notifications.push(text("No notifications").size(size).style(text::secondary));
        }
        for (i, reminder) in options.reminders.overrides.iter().enumerate() {
            notifications = notifications.push(
                column![
                    row![
                        crate::ui::controls::pick_list(
                            [Method::Notification, Method::Email],
                            Some(reminder.method),
                            move |v| message(Action::Method(i, v))
                        )
                        .text_size(size)
                        .width(Fill),
                        button(text("×").size(size))
                            .on_press(message(Action::Remove(i)))
                            .style(crate::ui::controls::ghost),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                    row![
                        crate::ui::controls::text_input(
                            "10",
                            &reminder.input.clone().unwrap_or_else(|| (reminder.minutes
                                / reminder.unit.factor())
                            .to_string())
                        )
                        .on_input(move |v| message(Action::Minutes(i, v)))
                        .size(size)
                        .width(80),
                        crate::ui::controls::pick_list(
                            [Unit::Minutes, Unit::Hours, Unit::Days, Unit::Weeks],
                            Some(reminder.unit),
                            move |v| message(Action::Unit(i, v))
                        )
                        .text_size(size),
                        text("before").size(size),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ]
                .spacing(6),
            );
        }
    }
    notifications = notifications.push(
        button(text("Add notification").size(size))
            .on_press_maybe((options.reminders.overrides.len() < 5).then_some(message(Action::Add)))
            .style(crate::ui::controls::ghost),
    );
    column![
        notifications,
        text("Availability").size(size).style(text::secondary),
        crate::ui::controls::pick_list(
            [Availability::Busy, Availability::Free],
            Some(options.transparency),
            move |v| message(Action::Availability(v))
        )
        .text_size(size)
        .width(Fill),
        text("Visibility").size(size).style(text::secondary),
        crate::ui::controls::pick_list(
            [Visibility::Default, Visibility::Public, Visibility::Private],
            Some(options.visibility),
            move |v| message(Action::Visibility(v))
        )
        .text_size(size)
        .width(Fill),
    ]
    .spacing(10)
    .width(Fill)
    .into()
}

/// Split save button with independent, generously sized pointer targets.
pub(super) fn event_save_control<'a>(
    state: &'a State,
    event: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    event_scope_control(state, event, metrics, false)
}

pub(super) fn event_scope_control<'a>(
    state: &'a State,
    event: &Event,
    metrics: &Metrics,
    deleting: bool,
) -> Element<'a, Message> {
    if event.id == crate::habits::NEW_HABIT_ID && !deleting {
        return button(
            text(if state.habits.busy {
                "Saving…"
            } else {
                "Save changes"
            })
            .size(metrics.label())
            .center()
            .width(Fill),
        )
        .width(Fill)
        .on_press_maybe(
            (!state.habits.busy)
                .then_some(Message::EventEdit(EventEditMessage::EventEditSaveRequested)),
        )
        .style(super::super::controls::primary)
        .into();
    }
    let saving = state
        .calendar
        .edit_draft
        .as_ref()
        .is_some_and(|draft| draft.saving);
    let busy = saving
        || state
            .calendar
            .edit_draft
            .as_ref()
            .is_some_and(|draft| draft.loading);
    let toggle = if deleting {
        Message::EventEdit(EventEditMessage::EventEditDeleteMenuToggled)
    } else {
        Message::EventEdit(EventEditMessage::EventEditSaveMenuToggled)
    };
    let request = if deleting {
        Message::Event(EventMessage::EventDeleteRequested(event.id.clone()))
    } else {
        Message::EventEdit(EventEditMessage::EventEditSaveRequested)
    };
    let scoped = |scope| {
        if deleting {
            Message::Event(EventMessage::EventDeleteScoped(event.id.clone(), scope))
        } else {
            Message::EventEdit(EventEditMessage::EventEditSaveScoped(scope))
        }
    };
    let height = 40.0;
    let main = button(
        text(if deleting {
            "Delete"
        } else if saving {
            "Saving…"
        } else {
            "Save current event"
        })
        .size(metrics.label())
        .center()
        .width(Fill),
    )
    .width(Fill)
    .height(height)
    .on_press_maybe((!busy).then_some(request))
    .style(super::super::view_half_style(!deleting));
    let arrow = button(text("▾").size(metrics.label()).center().width(Fill))
        .width(48.0)
        .height(height)
        .on_press_maybe((!busy).then_some(toggle.clone()))
        .style(super::super::view_half_style(!deleting));
    let divider = container(Space::new().height(24.0))
        .width(1.0)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.extended_palette().background.stronger.color.into()),
            ..Default::default()
        });
    let split = container(
        row![main, divider, arrow]
            .spacing(0)
            .align_y(Alignment::Center),
    )
    .clip(true)
    .style(super::super::view_button_style(!deleting));
    let open = state.calendar.edit_draft.as_ref().is_some_and(|draft| {
        if deleting {
            draft.delete_menu_open
        } else {
            draft.save_menu_open
        }
    });
    let choices = column![
        button(
            text(if deleting {
                "Delete current and future events"
            } else {
                "Save current and future events"
            })
            .size(metrics.label())
            .width(Fill)
        )
        .width(Fill)
        .padding(10)
        .on_press(scoped(calendar::SaveScope::CurrentAndFuture))
        .style(super::super::controls::secondary),
        button(
            text(if deleting {
                "Delete all events"
            } else {
                "Save all events"
            })
            .size(metrics.label())
            .width(Fill)
        )
        .width(Fill)
        .padding(10)
        .on_press(scoped(calendar::SaveScope::All))
        .style(super::super::controls::secondary),
    ]
    .spacing(4);
    let menu = container(choices)
        .padding(6)
        .width(Fill)
        .style(super::super::controls::surface);
    super::super::anchored_menu::anchored_menu(split, menu, open, toggle)
}

/// The same add/search/remove flow used for task projects, staged until Save.
pub(super) fn event_projects<'a>(
    state: &'a State,
    selection: &'a calendar::projects::Selection,
    metrics: &Metrics,
    message: fn(calendar::projects::Action) -> Message,
) -> Element<'a, Message> {
    use calendar::projects::Action;
    let mut content = column![
        text("Projects")
            .size(metrics.label())
            .style(text::secondary)
    ]
    .spacing(6)
    .width(Fill);
    if selection.ids.is_empty() {
        content = content.push(
            text("No projects")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for id in &selection.ids {
        let project = state.config.projects.get(*id);
        let name = project
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("Removed project ({id})"));
        let color = project
            .and_then(|p| p.color())
            .unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e));
        content = content.push(
            row![
                color_circle(color),
                text(name).size(metrics.label()).width(Fill),
                button(text("×").size(metrics.label()))
                    .on_press(message(Action::Remove(*id)))
                    .style(crate::ui::controls::ghost),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
    }
    content = content.push(
        button(
            text(if selection.open {
                "Done"
            } else {
                "+ Add project"
            })
            .size(metrics.label())
            .center()
            .width(Fill),
        )
        .width(Fill)
        .on_press(message(Action::Toggle))
        .style(crate::ui::controls::secondary),
    );
    if selection.open {
        content = content.push(
            crate::ui::controls::text_input("Search projects…", &selection.query)
                .on_input(move |query| message(Action::Query(query)))
                .size(metrics.label())
                .width(Fill),
        );
        let query = selection.query.trim().to_lowercase();
        let available: Vec<_> = state
            .config
            .projects
            .sorted()
            .into_iter()
            .filter(|p| !selection.ids.contains(&p.id))
            .filter(|p| p.name.to_lowercase().contains(&query))
            .collect();
        if available.is_empty() {
            content = content.push(
                text(if state.config.projects.items.is_empty() {
                    "Create a project in Projects first"
                } else if query.is_empty() {
                    "Every project is already added"
                } else {
                    "No matching projects"
                })
                .size(metrics.label())
                .style(text::secondary),
            );
        }
        let mut choices = column![].spacing(4);
        for project in available {
            choices = choices.push(
                row![
                    color_circle(
                        project
                            .color()
                            .unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e))
                    ),
                    button(text(&project.name).size(metrics.label()).width(Fill))
                        .width(Fill)
                        .on_press(message(Action::Add(project.id)))
                        .style(crate::ui::controls::ghost),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
        content = content.push(
            container(crate::ui::controls::scrollable(choices).height(Length::Shrink))
                .max_height(160),
        );
    }
    content.into()
}

/// A night has just two timestamps and independent confidence flags.
pub(super) fn sleep_editor<'a>(
    state: &'a State,
    event: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let Some(draft) = state.calendar.edit_draft.as_ref() else {
        return Space::new().into();
    };
    let Some(record) = draft.sleep.as_ref() else {
        return Space::new().into();
    };
    let mut preview = event.clone();
    preview.start = draft.start;
    preview.end = draft.end;
    let field = |label: &'static str, date: Element<'a, Message>, time: Element<'a, Message>| {
        column![
            text(label).size(metrics.label()),
            row![date, time].spacing(6)
        ]
        .spacing(4)
    };
    let mut body = column![
        titled(state, event, text("Sleep").size(metrics.label() + 3.0), metrics),
        text(format!("{} · {}", calendar::sleep::duration(&preview), calendar::sleep::time_label(&preview, state.config.appearance.time_format))).size(metrics.label()),
        field("Sleep start",
            crate::ui::controls::text_input("YYYY-MM-DD", &draft.date_text).on_input(|v0| Message::EventEdit(EventEditMessage::EventEditDateChanged(v0))).into(),
            crate::ui::controls::text_input("HH:MM", &draft.start_text).on_input(|v0| Message::EventEdit(EventEditMessage::EventEditStartChanged(v0))).into()),
        crate::ui::controls::checkbox(record.estimated_start).label("Estimated Sleep Start").on_toggle(|v0| Message::Settings(SettingsMessage::SleepEstimatedStart(v0))),
        field("Wake-up time",
            crate::ui::controls::text_input("YYYY-MM-DD", &draft.end_date_text).on_input(|v0| Message::Event(EventMessage::EventEndDateChanged(v0))).into(),
            crate::ui::controls::text_input("HH:MM", &draft.end_text).on_input(|v0| Message::EventEdit(EventEditMessage::EventEditEndChanged(v0))).into()),
        crate::ui::controls::checkbox(record.estimated_end).label("Estimated Sleep End").on_toggle(|v0| Message::Settings(SettingsMessage::SleepEstimatedEnd(v0))),
        text(if record.automatic {
            "Calculated from the first timed event. Saving records your times and preserves them when your schedule changes."
        } else { "Recorded sleep. These times are preserved when your schedule changes." }).size(metrics.label() - 1.0).style(text::secondary),
    ].spacing(metrics.spacing).width(Fill);
    if let Some(error) = &state.calendar.error {
        body = body.push(text(error).style(text::danger));
    }
    let mut save = crate::ui::controls::action_button(
        text(if draft.saving {
            "Saving…"
        } else {
            "Save sleep"
        })
        .center()
        .width(Fill),
    )
    .width(Fill);
    if !draft.saving {
        save = save.on_press(Message::Settings(SettingsMessage::SleepSaveRequested));
    }
    container(
        column![
            crate::ui::controls::scrollable(container(body).padding(iced::Padding {
                right: 14.0,
                ..iced::Padding::ZERO
            }))
            .height(Fill),
            save,
            button(text("Close").center().width(Fill))
                .width(Fill)
                .on_press(Message::Event(EventMessage::EventClosed))
                .style(crate::ui::controls::secondary),
        ]
        .spacing(metrics.spacing)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .style(crate::ui::controls::surface)
    .into()
}

/// A habit an event can count towards, in the event editor's picker.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HabitChoice {
    id: Option<String>,
    name: String,
}

impl std::fmt::Display for HabitChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// Which habit this event counts towards. A new event with no choice gets the
/// habit named after it when it is saved, created if need be.
fn event_habit_picker<'a>(
    state: &'a State,
    event: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let new = event.id == crate::habits::NEW_HABIT_ID;
    let mut options = vec![HabitChoice {
        id: None,
        name: if new {
            "Match the title".to_owned()
        } else {
            "None".to_owned()
        },
    }];
    options.extend(
        state
            .habits
            .library
            .habits
            .iter()
            .filter(|habit| !habit.archived && habit.kind != timeslay_core::habits::Kind::No)
            .map(|habit| HabitChoice {
                id: Some(habit.id.clone()),
                name: habit.name.clone(),
            }),
    );
    let current = if new {
        event.details.habit_id.clone()
    } else {
        state.habits.habit_of(event).map(str::to_owned)
    };
    let selected = options
        .iter()
        .find(|option| option.id == current)
        .cloned()
        .or_else(|| options.first().cloned());
    column![
        text("Habit").size(metrics.label()).style(text::secondary),
        crate::ui::controls::pick_list(options, selected, |choice: HabitChoice| {
            Message::Habits(crate::habits::Action::EventHabitChosen(choice.id))
        })
        .text_size(metrics.label())
        .width(Fill),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}
