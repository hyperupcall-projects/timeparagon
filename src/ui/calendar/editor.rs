//! The panel that edits an event that already exists.

use super::*;
use iced::widget::column;

fn high_contrast(theme: &Theme) -> bool {
	use timeparagon_core::theme::Scheme;
	[
		Scheme::LightHighContrast.palette(),
		Scheme::DarkHighContrast.palette(),
	]
	.contains(&theme.palette())
}

/// A quiet inset card keeps related fields together on light and dark themes.
pub(crate) fn form_section<'a>(
	title: &'static str,
	fields: impl Into<Element<'a, Message>>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	container(
		column![
			crate::ui::controls::field_label(title, metrics.label()).style(|theme: &Theme| {
				if high_contrast(theme) {
					text::default(theme)
				} else {
					text::secondary(theme)
				}
			}),
			iced::widget::rule::horizontal(1).style(|theme: &Theme| {
				let mut style = iced::widget::rule::default(theme);
				if high_contrast(theme) {
					style.color = theme.extended_palette().background.stronger.color;
				}
				style
			}),
			fields.into(),
		]
		.spacing(10)
		.width(Fill),
	)
	.padding(12)
	.width(Fill)
	.style(|theme: &Theme| {
		let palette = theme.extended_palette();
		if high_contrast(theme) {
			return container::Style {
				background: Some(palette.background.weaker.color.into()),
				border: iced::Border {
					color: palette.background.stronger.color,
					width: 2.0,
					radius: 8.0.into(),
				},
				..Default::default()
			};
		}
		container::Style {
			background: Some(
				Color {
					a: 0.025,
					..palette.background.base.text
				}
				.into(),
			),
			border: iced::Border {
				color: Color {
					a: 0.10,
					..palette.background.base.text
				},
				width: 1.0,
				radius: 8.0.into(),
			},
			..Default::default()
		}
	})
	.into()
}

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
	let label = |name: &'static str| crate::ui::controls::field_label(name, metrics.label());

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
		!event.is_habit() && !crate::timeblocking::is_block(event, &state.config.calendar);
	let mut start = column![label(if event.is_habit() {
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
	let mut scheduling = column![start].spacing(metrics.spacing).width(Fill);
	if !event.is_habit() {
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
		scheduling = scheduling.push(end);
	}
	scheduling = scheduling.push(super::super::recurrence::editor(
		&draft.rule,
		draft.start.date_naive(),
		state.config.calendar.first_day_of_week,
		metrics,
		false,
		|v| Message::EventEdit(EventEditMessage::EventEditRecurrence(v)),
	));
	if event.is_calendar_task() && !event.is_habit() {
		scheduling = scheduling.push(
			column![
				label("Deadline"),
				super::super::controls::date(
					state.config.appearance.time_format,
					&draft.deadline_text,
					true,
					true,
					!draft.saving,
					metrics.label(),
					|value| Message::EventEdit(EventEditMessage::EventEditDeadlineChanged(value)),
				),
			]
			.spacing(6)
			.width(Fill),
		);
	}
	scheduling = scheduling.push(event_notifications(&draft.options, metrics, |v| {
		Message::EventEdit(EventEditMessage::EventEditOptions(v))
	}));
	let mut information = column![super::super::description::preview(
		&event.id,
		&draft.description,
		metrics,
	)]
	.spacing(metrics.spacing);
	let new_habit = event.id == crate::habits::NEW_HABIT_ID;
	if !new_habit {
		if let Some(target) = crate::notes::target_of(state, event) {
			information = information.push(super::super::notes::section(state, target, metrics));
		} else {
			information = information.push(
				text("Choose a habit above to keep notes on it.")
					.size(metrics.label())
					.style(text::secondary),
			);
		}
	}
	let mut body = column![
		titled(state, event, title, metrics),
		form_section(
			"Scheduling",
			super::super::timeblocking::scheduled_by_block(state, event, scheduling, metrics),
			metrics
		),
		form_section("Information", information, metrics),
	]
	.spacing(metrics.spacing)
	.width(Fill);
	let mut organization = column![item_color_picker(
		&draft.color_input,
		metrics,
		|v| Message::EventEdit(EventEditMessage::ColorChanged(v)),
		|v| Message::EventEdit(EventEditMessage::ColorChanged(v))
	)]
	.spacing(metrics.spacing)
	.width(Fill);
	if let Some(source) = source {
		organization = organization.push(source);
	}
	organization = organization.push(event_projects(state, &draft.projects, metrics, |v| {
		Message::EventEdit(EventEditMessage::EventEditProjects(v))
	}));
	if event.is_habit() {
		organization = organization.push(event_habit_picker(state, event, metrics));
	} else {
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
		organization = organization.push(labels);
	}

	if !new_habit {
		let place = crate::travel::place_of(&state.config.travel, event)
			.and_then(|id| state.config.travel.place(id));
		if place.is_none() && !event.details.location.is_empty() {
			organization = organization.push(detail_line(
				"Location",
				event.details.location.clone(),
				metrics,
			));
		}
		organization =
			organization
				.push(label("Location"))
				.push(super::super::travel::location_control(
					state,
					crate::travel::PickTarget::Event(event.id.clone()),
					place,
					"Add location",
					metrics,
				));
	}
	body = body.push(form_section("Organization", organization, metrics));
	if !event.is_task() && !event.details.habit && !new_habit {
		let mut meeting = column![].spacing(metrics.spacing).width(Fill);
		for link in super::super::meetings::links(event) {
			meeting = meeting.push(super::super::meetings::join_button(link, metrics));
		}
		if !crate::timeblocking::is_block(event, &state.config.calendar) {
			meeting = meeting.push(meeting_section(
				state,
				&draft.meeting,
				Some(event),
				metrics,
				|v| Message::EventEdit(EventEditMessage::EventEditMeeting(v)),
			));
		}
		if !event.details.organizer.is_empty() {
			meeting = meeting.push(detail_line(
				"Organizer",
				event.details.organizer.clone(),
				metrics,
			));
		}
		if !event.details.attendees.is_empty() && event.details.attendance.guests.is_empty() {
			meeting = meeting.push(detail_line(
				"Guests",
				event.details.attendees.join("\n"),
				metrics,
			));
		}
		meeting = meeting.push(event_presence(&draft.options, metrics, |v| {
			Message::EventEdit(EventEditMessage::EventEditOptions(v))
		}));
		body = body.push(form_section("Meeting", meeting, metrics));
	}
	if let Some(error) = &state.calendar.error {
		body = body.push(text(error).size(metrics.label()).style(text::danger));
	}

	if state.calendar.open_event.as_deref() != Some(event.id.as_str()) {
		body = body.push(event_save_control(state, event, metrics));
	}
	body.into()
}

/// Combined options for callers that do not lay out separate sections.
pub(super) fn event_options<'a>(
	options: &'a calendar::event_options::EventOptions,
	metrics: &Metrics,
	message: fn(calendar::event_options::Action) -> Message,
) -> Element<'a, Message> {
	column![
		event_notifications(options, metrics, message),
		event_presence(options, metrics, message)
	]
	.spacing(10)
	.into()
}

pub(super) fn event_notifications<'a>(
	options: &'a calendar::event_options::EventOptions,
	metrics: &Metrics,
	message: fn(calendar::event_options::Action) -> Message,
) -> Element<'a, Message> {
	use calendar::event_options::{Action, Method, Unit};
	let size = metrics.label();
	let mut notifications = column![
		crate::ui::controls::field_label("Notifications", size),
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
	notifications.into()
}

pub(super) fn event_presence<'a>(
	options: &'a calendar::event_options::EventOptions,
	metrics: &Metrics,
	message: fn(calendar::event_options::Action) -> Message,
) -> Element<'a, Message> {
	use calendar::event_options::{Action, Availability, Visibility};
	let size = metrics.label();
	column![
		crate::ui::controls::field_label("Availability", size),
		crate::ui::controls::pick_list(
			[Availability::Busy, Availability::Free],
			Some(options.transparency),
			move |v| message(Action::Availability(v))
		)
		.text_size(size)
		.width(Fill),
		crate::ui::controls::field_label("Visibility", size),
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
	let request = if deleting {
		Message::Event(EventMessage::EventDeleteRequested(event.id.clone()))
	} else {
		Message::EventEdit(EventEditMessage::EventEditSaveRequested)
	};
	let height = 40.0;
	// "Current and future" and "all" only mean something for a series. Sleep
	// uses "current and future" to change its shared schedule, and a habit is
	// always a series even before its master has loaded.
	let scoped_choices =
		event.is_recurring() || event.details.habit || event.details.sleep.is_some();
	// A time block can take the tasks and habits planned in it along.
	let with_tasks = deleting && crate::timeblocking::is_block(event, &state.config.calendar);
	// A single block only offers that when something is planned in it.
	let single_with_tasks =
		with_tasks && !scoped_choices && !crate::timeblocking::assigned(state, event).is_empty();
	if !scoped_choices && !single_with_tasks {
		let single = button(
			text(if deleting {
				"Delete"
			} else if saving {
				"Saving…"
			} else {
				"Save"
			})
			.size(metrics.label())
			.center()
			.width(Fill),
		)
		.width(Fill)
		.height(height)
		.on_press_maybe((!busy).then_some(request))
		.style(super::super::view_half_style(!deleting));
		return container(single)
			.clip(true)
			.style(super::super::view_button_style(!deleting))
			.into();
	}
	let toggle = if deleting {
		Message::EventEdit(EventEditMessage::EventEditDeleteMenuToggled)
	} else {
		Message::EventEdit(EventEditMessage::EventEditSaveMenuToggled)
	};
	let scoped = |scope| {
		if deleting {
			Message::Event(EventMessage::EventDeleteScoped(event.id.clone(), scope))
		} else {
			Message::EventEdit(EventEditMessage::EventEditSaveScoped(scope))
		}
	};
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
	let choice = |label: &'static str, message: Message| {
		button(text(label).size(metrics.label()).width(Fill))
			.width(Fill)
			.padding(10)
			.on_press(message)
			.style(super::super::controls::secondary)
	};
	let with_tasks_choice = |label, scope| {
		with_tasks.then(|| {
			choice(
				label,
				Message::Event(EventMessage::EventDeleteWithTasks(event.id.clone(), scope)),
			)
		})
	};
	let choices = if single_with_tasks {
		column![choice(
			"Delete including planned tasks",
			Message::Event(EventMessage::EventDeleteWithTasks(
				event.id.clone(),
				calendar::SaveScope::Current,
			)),
		)]
	} else {
		column![choice(
			if deleting {
				"Delete current and future events"
			} else {
				"Save current and future events"
			},
			scoped(calendar::SaveScope::CurrentAndFuture),
		)]
		.push(with_tasks_choice(
			"Delete current and future events including planned tasks",
			calendar::SaveScope::CurrentAndFuture,
		))
		.push(choice(
			if deleting {
				"Delete all events"
			} else {
				"Save all events"
			},
			scoped(calendar::SaveScope::All),
		))
		.push(with_tasks_choice(
			"Delete all events including planned tasks",
			calendar::SaveScope::All,
		))
	}
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
	let mut content = column![crate::ui::controls::field_label(
		"Projects",
		metrics.label()
	)]
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
	let field = |label: &'static str, day: NaiveDate, time: Element<'a, Message>| {
		column![
			crate::ui::controls::field_label(label, metrics.label()),
			text(format!(
				"{}, {}",
				calendar::month_day_label(day),
				day.year()
			))
			.size(metrics.label())
			.style(text::secondary),
			time,
		]
		.spacing(4)
		.width(Fill)
	};
	let mut body = column![
        titled(state, event, text("Sleep").size(metrics.label() + 6.0).font(Font {
            weight: Weight::Bold,
            ..Font::DEFAULT
        }), metrics),
        text(calendar::sleep::duration(&preview)).size(metrics.label() + 10.0),
        text(if record.automatic {
            "Planned from your weekly sleep schedule. Save changes for this night or for this and future matching weekdays."
        } else { "Recorded sleep. These times are preserved when your schedule changes." }).size(metrics.label() - 1.0).style(text::secondary),
        form_section(
            "Scheduling",
            column![
                field("Bedtime", draft.start.date_naive(),
                    crate::ui::controls::time(
                        state.config.appearance.time_format,
                        &draft.start_text,
                        !draft.saving,
                        metrics.label(),
                        |v0| Message::EventEdit(EventEditMessage::EventEditStartChanged(v0)),
                    )),
                field("Wake-up time", draft.end.date_naive(),
                    crate::ui::controls::time(
                        state.config.appearance.time_format,
                        &draft.end_text,
                        !draft.saving,
                        metrics.label(),
                        |v0| Message::EventEdit(EventEditMessage::EventEditEndChanged(v0)),
                    )),
            ]
            .spacing(metrics.spacing)
            .width(Fill),
            metrics,
        ),
    ].spacing(metrics.spacing).width(Fill);
	body = body.push(actual_sleep(state, draft, record, &preview, metrics));
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
	// The timer works from the saved night, so it is offered from what is on
	// Google rather than from unsaved edits.
	let timer = calendar::sleep::timer(event, crate::clock::now()).map(|timer| {
		let mut button =
			crate::ui::controls::action_button(text(timer.label()).center().width(Fill))
				.width(Fill);
		if !draft.saving {
			button = button.on_press(Message::Settings(SettingsMessage::SleepTimerPressed(
				event.id.clone(),
			)));
		}
		Element::from(button)
	});
	container(
		column![
			crate::ui::controls::scrollable(container(body).padding(iced::Padding {
				right: 14.0,
				..iced::Padding::ZERO
			}))
			.height(Fill),
		]
		.push(timer)
		.push(
			column![
				save,
				button(text("Close").center().width(Fill))
					.width(Fill)
					.on_press(Message::Event(EventMessage::EventClosed))
					.style(crate::ui::controls::secondary),
			]
			.spacing(metrics.spacing),
		)
		.spacing(metrics.spacing)
		.height(Fill),
	)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.style(crate::ui::controls::surface)
	.into()
}

/// The night's actual sleep, editable, and the wake-up times that would give
/// 6 to 9 hours and the planned amount.
fn actual_sleep<'a>(
	state: &'a State,
	draft: &'a calendar::EditDraft,
	record: &calendar::sleep::Record,
	planned: &Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let format = state.config.appearance.time_format;
	let summary = match (record.actual_start, record.actual_end) {
		(Some(start), Some(end)) => format!(
			"Slept {} (planned {})",
			calendar::sleep::minutes_label((end - start).num_minutes()),
			calendar::sleep::duration(planned)
		),
		(Some(start), None) => format!(
			"Sleeping since {} · {} so far",
			calendar::time_format::clock(start, format),
			calendar::sleep::minutes_label((crate::clock::now() - start).num_minutes())
		),
		_ => "Not recorded yet. Use Start sleep at bedtime, or enter the times.".to_owned(),
	};
	let field = |label: &'static str, value: &'a str, change: fn(String) -> SettingsMessage| {
		let mut controls = row![crate::ui::controls::time(
			format,
			value,
			!draft.saving,
			metrics.label(),
			move |v0| Message::Settings(change(v0)),
		)]
		.spacing(6)
		.align_y(Alignment::Center);
		if !value.is_empty() && !draft.saving {
			controls = controls.push(
				button(text("Clear").size(metrics.label() - 1.0))
					.on_press(Message::Settings(change(String::new())))
					.style(crate::ui::controls::ghost),
			);
		}
		column![
			crate::ui::controls::field_label(label, metrics.label()),
			controls
		]
		.spacing(4)
		.width(Fill)
	};
	let mut preview = planned.clone();
	preview.details.sleep = Some(record.clone());
	let mut predictions = column![].spacing(2);
	for (label, at) in calendar::sleep::wake_predictions(&preview) {
		predictions = predictions.push(
			row![
				text(label).size(metrics.label()).width(Fill),
				text(calendar::time_format::clock(at, format)).size(metrics.label()),
			]
			.spacing(6),
		);
	}
	let basis = if record.actual_start.is_some() {
		"Counted from when sleep actually started."
	} else {
		"Counted from the planned bedtime."
	};
	column![
		form_section(
			"Actual sleep",
			column![
				text(summary).size(metrics.label()).style(text::secondary),
				field(
					"Actual sleep start",
					&draft.actual_start_text,
					SettingsMessage::SleepActualStartChanged
				),
				field(
					"Actual wake-up",
					&draft.actual_end_text,
					SettingsMessage::SleepActualEndChanged
				),
			]
			.spacing(metrics.spacing)
			.width(Fill),
			metrics,
		),
		form_section(
			"Wake up at",
			column![
				predictions,
				text(basis)
					.size(metrics.label() - 1.0)
					.style(text::secondary),
			]
			.spacing(metrics.spacing)
			.width(Fill),
			metrics,
		),
	]
	.spacing(metrics.spacing)
	.width(Fill)
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
	if event.is_task() {
		return super::super::timeblocking::habit_link(state, event, metrics);
	}
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
			.filter(|habit| !habit.archived && habit.kind != timeparagon_core::habits::Kind::No)
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
		crate::ui::controls::field_label("Habit", metrics.label()),
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

/// Google palette plus a custom RGB input, shared by item editors.
pub(crate) fn item_color_picker<'a>(
	input: &'a str,
	metrics: &Metrics,
	changed: impl Fn(String) -> Message + 'a,
	picked: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
	let selected = calendar::colors::resolve(input);
	let mut rows = column![].spacing(6);
	for chunk in calendar::colors::palette().chunks(6) {
		let swatches: Vec<Element<'a, Message>> = chunk
			.iter()
			.map(|&color| {
				let id = calendar::colors::event_id(color).unwrap().to_owned();
				button(
					text(if selected == Some(color) { "✓" } else { " " })
						.center()
						.color(calendar::colors::on_color(color)),
				)
				.width(28)
				.height(28)
				.padding(0)
				.style(move |_, _| button::Style {
					background: Some(color.into()),
					..Default::default()
				})
				.on_press(picked(id))
				.into()
			})
			.collect();
		rows = rows.push(row(swatches).spacing(6));
	}
	let custom = if input.starts_with('#') { input } else { "" };
	let field = crate::ui::controls::text_input("#336699", custom)
		.on_input(changed)
		.on_submit(picked(input.to_owned()))
		.size(metrics.label());
	let error = calendar::colors::selection(input).err();
	let mut body = column![
		text("Color").size(metrics.label()),
		rows,
		row![
			field,
			crate::ui::controls::quiet_button("Apply").on_press(picked(input.to_owned()))
		]
		.spacing(6),
		crate::ui::controls::quiet_button("Use default color").on_press(picked(String::new())),
		text("Custom colors sync to Google Calendar on calendars you own.")
			.size(metrics.label() - 2.0)
			.style(text::secondary),
	]
	.spacing(6);
	if let Some(error) = error {
		body = body.push(text(error).size(metrics.label() - 1.0).style(text::danger));
	}
	body.into()
}
