use super::layout::Metrics;
use crate::app::{EventMessage, TaskMessage};
use crate::calendar::time_format::TimeFormat;
use crate::{
	app::{Message, State},
	calendar::Event,
	timeblocking::{self as tb, Action},
};
use iced::widget::{button, column, container, mouse_area, rich_text, row, span, text};
use iced::{Color, Element, Fill, Length};
fn msg(a: impl Into<Message>) -> Message {
	a.into()
}
pub fn task_row<'a>(
	state: &'a State,
	task: &'a Event,
	metrics: &Metrics,
	backlog: bool,
) -> Element<'a, Message> {
	let m = tb::metadata(task);
	let overdue = tb::overdue(task);
	let over_capacity = tb::block_of(task, &state.calendar.events, tb::day_start_hour(state))
		.is_some_and(|block| {
			let tasks = tb::assigned(state, block);
			let mut total = 0u64;
			for item in tasks {
				total += u64::from(tb::metadata(item).minutes);
				if item.id == task.id {
					break;
				}
			}
			total > (block.end - block.start).num_minutes().max(0) as u64
		});
	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));
	let mut check =
		button(text(if task.completed { "●" } else { "○" })).style(crate::ui::controls::ghost);
	if state.task_operations.busy.is_empty() {
		check = check.on_press(Message::Task(TaskMessage::TaskToggled(task.id.clone())));
	}
	let mut title_content = column![task_title(task, metrics.label())].spacing(2);
	if m.habit_id.is_some() {
		title_content = title_content.push(
			text("Habit")
				.size(metrics.label() - 2.0)
				.style(text::secondary),
		);
	}
	let status = if busy {
		Some("Saving…")
	} else {
		match tb::task_status(task) {
			tb::TaskStatus::Completed => Some("Completed ✓"),
			tb::TaskStatus::Ignored => Some("Ignored"),
			tb::TaskStatus::Active => None,
		}
	};
	if let Some(status) = status {
		title_content = title_content.push(
			text(status)
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		);
	}
	let title = button(title_content)
		.width(Fill)
		.style(crate::ui::controls::ghost)
		.on_press(Message::Event(EventMessage::EventOpened(task.id.clone())));
	let handle = text("⠿").size(17);
	let mut line = row![handle, check].spacing(4).align_y(iced::Center);
	// Where the task came from, if a service we know put it here.
	if let Some(source) = crate::integrations::source_in(&task.details.description) {
		line = line.push(super::source_icon::link(&source, 14.0));
	}
	line = line
		.push(title)
		.push(estimate_button(state, task, metrics.label()));
	if backlog {
		line = line.push(
			button(text("Reschedule").size(metrics.label()))
				.on_press(Message::Event(EventMessage::EventOpened(task.id.clone())))
				.style(crate::ui::controls::ghost),
		);
	}
	let mut content = column![line].spacing(3);
	if !m.labels.is_empty() {
		content = content.push(
			text(
				m.labels
					.iter()
					.map(|label| format!("#{label}"))
					.collect::<Vec<_>>()
					.join(" "),
			)
			.size(metrics.label() - 2.0)
			.style(text::secondary),
		);
	}
	if backlog {
		content = content.push(
			text(tb::task_list_name(state, &task.calendar_id))
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		);
	}
	if backlog {
		if let Some(due) = tb::due(task) {
			let late = due.is_past(crate::clock::now().naive_local());
			content = content.push(
				text(due_label(&due, state.config.appearance.time_format))
					.size(metrics.label() - 1.0)
					.style(if late { text::danger } else { text::secondary }),
			);
		}
	} else if let Some(deadline) = crate::calendar::deadline::get(task) {
		content = content.push(
			text(deadline.to_string())
				.size(metrics.label() - 1.0)
				.style(if overdue {
					text::danger
				} else {
					text::secondary
				}),
		);
	}
	let card = container(content)
		.padding([5, 6])
		.width(Fill)
		.style(move |theme: &iced::Theme| {
			let palette = theme.extended_palette();
			container::Style {
				background: Some(
					if over_capacity {
						palette.danger.weak.color
					} else {
						palette.background.weakest.color
					}
					.into(),
				),
				border: iced::Border {
					color: if overdue {
						Color::from_rgb8(220, 65, 55)
					} else {
						palette.background.strong.color
					},
					width: 1.0,
					radius: crate::ui::controls::RADIUS_ROW.into(),
				},
				..crate::ui::controls::row_surface(theme)
			}
		});
	crate::ui::controls::locked(
		super::task_drag::task_drag(card, &task.id, state.timeblocking.dragging.as_deref()),
		task.details.sending,
	)
}

/// The floating card that follows the pointer while a task is in the air.
///
/// It is drawn above everything else but never takes the pointer, so the drop
/// targets underneath still see the release. The border uses the theme's
/// primary color to echo the highlight on the places it can be dropped.
pub(super) fn drag_ghost<'a>(
	state: &'a State,
	task: &'a Event,
	metrics: &Metrics,
	width: f32,
) -> Element<'a, Message> {
	let m = tb::metadata(task);
	let mut body = column![
		row![
			text("⠿").size(metrics.label() + 2.0).style(text::secondary),
			task_title(task, metrics.label()),
		]
		.spacing(6)
		.align_y(iced::Center),
		text(tb::task_list_name(state, &task.calendar_id))
			.size(metrics.label() - 1.0)
			.style(text::secondary),
	]
	.spacing(3);
	if m.minutes > 0 {
		body = body.push(
			text(format!("Estimate: {}m", m.minutes))
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		);
	}
	container(body)
		.padding([6, 8])
		.width(Length::Fixed(width))
		.style(|theme: &iced::Theme| {
			let palette = theme.extended_palette();
			container::Style {
				background: Some(palette.background.weakest.color.into()),
				text_color: Some(palette.background.base.text),
				border: iced::Border {
					color: palette.primary.base.color,
					width: 2.0,
					radius: crate::ui::controls::RADIUS_ROW.into(),
				},
				shadow: crate::ui::controls::shadow(theme, 0.26, 8.0, 22.0),
				..container::Style::default()
			}
		})
		.into()
}

/// "Due Today", "Due Tomorrow", or "Due Mon, Oct 26" (with the year when it
/// isn't this one), followed by the time when the deadline has one.
fn due_label(due: &crate::calendar::deadline::Deadline, time_format: TimeFormat) -> String {
	let today = crate::clock::now().date_naive();
	let day = if due.date == today {
		"Today".to_owned()
	} else if today.succ_opt() == Some(due.date) {
		"Tomorrow".to_owned()
	} else if chrono::Datelike::year(&due.date) == chrono::Datelike::year(&today) {
		due.date.format("%a, %b %-d").to_string()
	} else {
		due.date.format("%a, %b %-d, %Y").to_string()
	};
	match due.time {
		Some(time) => format!(
			"Due {day} at {}",
			crate::calendar::time_format::clock(time, time_format)
		),
		None => format!("Due {day}"),
	}
}

pub(super) fn task_title<'a>(task: &'a Event, size: f32) -> Element<'a, Message> {
	let font = crate::ui::controls::pending_font(iced::Font::DEFAULT, task.details.sending);
	if tb::task_status(task) == tb::TaskStatus::Ignored {
		rich_text::<(), _, _, _>([span(task.title.as_str()).strikethrough(true).font(font)])
			.size(size)
			.width(Fill)
			.style(text::secondary)
			.into()
	} else {
		text(&task.title)
			.size(size)
			.font(font)
			.width(Fill)
			.style(if task.completed {
				text::secondary
			} else {
				text::default
			})
			.into()
	}
}
/// The title occupies its own field only while it is being edited.
pub(super) fn editable_title<'a>(
	state: &'a State,
	task: &'a Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	if let Some(habit) = state
		.habits
		.habit_of(task)
		.and_then(|id| state.habits.library.habit(id))
	{
		return text(&habit.name)
			.size(metrics.label() + 6.0)
			.font(iced::Font {
				weight: iced::font::Weight::Bold,
				..iced::Font::DEFAULT
			})
			.into();
	}

	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));
	let size = metrics.label() + 6.0;
	let title: Element<'a, Message> = if state.task_editor().editing_title {
		let mut input = crate::ui::controls::text_input("Task name", &state.task_editor().title)
			.id(iced::widget::Id::new("task-title"))
			.size(size);
		if !busy {
			input = input
				.on_input(|v| msg(crate::task_editor::Action::Title(v)))
				.on_submit(msg(crate::task_editor::Action::SaveTitle(task.id.clone())));
		}
		input.into()
	} else {
		let mut title = button(text(&task.title).size(size).font(iced::Font {
			weight: iced::font::Weight::Bold,
			..iced::Font::DEFAULT
		}))
		.padding(0)
		.width(Fill)
		.style(crate::ui::controls::ghost);
		if !busy {
			title = title.on_press(msg(crate::task_editor::Action::EditTitle));
		}
		title.into()
	};
	let mut body = column![title].spacing(4);
	if tb::metadata(task).habit_id.is_some() {
		body = body.push(
			text("Habit")
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		);
	}
	if state.task_editor().editing_title {
		body = body.push(
			text("Press Enter to save")
				.size(metrics.label() - 2.0)
				.style(text::secondary),
		);
	}
	if let Some(error) = &state.task_editor().title_error {
		body = body.push(text(error).size(metrics.label()).style(text::danger));
	}
	body.into()
}

/// A compact reference with separate examples and controls so it remains readable
/// at the width of the task popup.
fn quick_add_help(metrics: &Metrics) -> Element<'static, Message> {
	let size = metrics.label();
	let bold = iced::Font {
		weight: iced::font::Weight::Bold,
		..iced::Font::DEFAULT
	};
	let example = move |syntax: &'static str, description: &'static str| {
		column![
			text(syntax).size(size).font(bold),
			text(description).size(size).style(text::secondary),
		]
		.spacing(2)
	};
	let body = column![
		text("Quick-add syntax").size(size + 2.0).font(bold),
		text(
			"Click a highlight to keep its words as regular title text instead of applying task details."
		)
		.size(size),
		text("Task details").size(size).font(bold),
		example(
			"@label   ·   {@deep work}",
			"Add a label. Use braces for names with spaces."
		),
		example("#Project   ·   {#Project name}", "Add an existing project."),
		example("p1 / !1   ·   p2 / !2", "Set priority 1 or 2."),
		example("~30m", "Set an estimate in minutes."),
		iced::widget::rule::horizontal(1),
		text("Dates").size(size).font(bold),
		example(
			"today   ·   tomorrow   ·   monday",
			"Also: next monday, next week, in 3 days, in 2 weeks."
		),
		example(
			"oct 8   ·   October 8 2027",
			"Also: 8 oct, 2027-10-08. Dates without a year use the next occurrence."
		),
		example(
			"{oct 8}",
			"Braces set the deadline. Bare dates set the due date."
		),
		iced::widget::rule::horizontal(1),
		text("Editing highlights").size(size).font(bold),
		example(
			"Tab inside a highlight",
			"Apply its task details and remove the words."
		),
		example("Space", "Keep typing; detected words stay highlighted."),
		example(
			"Enter",
			"Save the task and remove all remaining highlighted syntax."
		),
		text("Unknown syntax stays in the title.")
			.size(size)
			.style(text::secondary),
	]
	.spacing(10)
	.padding(iced::Padding::from([0, 6]))
	.width(Fill);
	container(crate::ui::controls::scrollable(body).height(Length::Shrink))
		.max_height(460)
		.padding(14)
		.width(Fill)
		.style(super::controls::surface)
		.into()
}

/// The task fields used by details and every creation entry point.
pub fn fields<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
	let draft = state
		.inbox
		.draft
		.as_ref()
		.filter(|_| task.id == crate::inbox::DRAFT_ID);
	let title: Element<'a, Message> = if let Some(draft) = draft {
		let mut input = crate::ui::controls::text_input("Task, or paste a link", &draft.title)
			.id(iced::widget::Id::new(crate::inbox::TITLE_FIELD))
			.size(metrics.label() + 6.0);
		if !draft.saving {
			input = input
				.on_input(|v| Message::Inbox(crate::inbox::Action::Title(v)))
				.on_submit(Message::Inbox(crate::inbox::Action::Create));
		}
		let tokens = if draft.saving {
			Vec::new()
		} else {
			draft.quick_tokens(&state.config.projects)
		};
		let input =
			super::task_input::quick_input(input, &draft.title, tokens, metrics.label() + 6.0);
		let help = super::anchored_menu::anchored_menu(
			row![
				field_label("Title", metrics),
				iced::widget::Space::new().width(Fill),
				// No taller than the caption, so the title sits where every
				// other type's does.
				crate::ui::controls::quiet_button(text("?").size(metrics.label()))
					.padding([0, 8])
					.on_press(Message::Inbox(crate::inbox::Action::SyntaxHelp(
						!draft.syntax_help
					)))
			]
			.align_y(iced::Center),
			quick_add_help(metrics),
			draft.syntax_help,
			Message::Inbox(crate::inbox::Action::SyntaxHelp(false)),
		);
		column![help, crate::voice::input(state, input)]
			.spacing(6)
			.into()
	} else {
		editable_title(state, task, metrics)
	};
	let description: Element<'a, Message> = if let Some(draft) = draft {
		let mut input = crate::ui::controls::text_editor(&state.task_description)
			.placeholder("Add description…")
			.size(metrics.label())
			.height(140);
		if !draft.saving {
			input = input.on_action(|a| Message::Inbox(crate::inbox::Action::Description(a)));
		}
		let mut description = column![field_label("Description", metrics), input].spacing(6);
		if let Some(links) =
			super::links::for_editor(&state.task_description.text(), metrics.label())
		{
			description = description.push(links);
		}
		description.into()
	} else {
		super::description::preview(&task.id, &task.details.description.clone(), metrics)
	};
	let mut information = column![description].spacing(metrics.spacing);
	if task.is_habit() {
		information = information.push(habit_link(state, task, metrics));
	}
	if task.id != crate::inbox::DRAFT_ID {
		if let Some(target) = crate::notes::target_of(state, task) {
			information = information.push(super::notes::section(state, target, metrics));
		}
	}
	let busy =
		state.task_operations.busy.contains_task(&task.id) || draft.is_some_and(|d| d.saving);
	let color_input = state
		.task_editor()
		.color_input
		.as_ref()
		.filter(|(id, _)| id == &task.id)
		.map(|(_, value)| value.as_str())
		.or_else(|| {
			task.details
				.task_data
				.as_ref()
				.and_then(|d| d.metadata.color.as_deref())
		})
		.or(task.details.color_id.as_deref())
		.unwrap_or("");
	let mut body = column![].spacing(6);
	if !busy {
		body = body.push(crate::ui::calendar::item_color_picker(
			color_input,
			metrics,
			move |v| msg(crate::task_editor::Action::ColorInput(task.id.clone(), v)),
			move |v| msg(crate::task_editor::Action::SaveColor(task.id.clone(), v)),
		));
	}

	// A new task's form has a type selector that already says what it is.
	let title = if draft.is_some() {
		title
	} else {
		super::calendar::titled(state, task, title, metrics)
	};
	column![
		title,
		super::calendar::form_section(
			"Scheduling",
			scheduled_by_block(
				state,
				task,
				column![
					due_fields(state, task, metrics),
					deadline_field(state, task, metrics),
				]
				.spacing(12),
				metrics,
			),
			metrics
		),
		super::calendar::form_section("Information", information, metrics),
		super::calendar::form_section(
			"Organization",
			column![body, organization_fields(state, task, metrics)].spacing(metrics.spacing),
			metrics
		),
	]
	.spacing(metrics.spacing)
	.width(Fill)
	.into()
}

/// A task in a time block takes its schedule from the block, so its own
/// scheduling fields are shown grayed out. One repeating with the block
/// repeats as the block does, and can only stop doing so; a one-off in a
/// repeating block can start repeating with it.
pub fn scheduled_by_block<'a>(
	state: &State,
	task: &Event,
	content: impl Into<Element<'a, Message>>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let change = |label: &'a str, action: Action| {
		button(text(label).size(metrics.label()))
			.style(crate::ui::controls::secondary)
			.on_press(msg(action))
	};
	let with_button = |note: &'a str, content, button| -> Element<'a, Message> {
		column![
			super::controls::owned_elsewhere(note, content, metrics.label()),
			button,
		]
		.spacing(8)
		.width(Fill)
		.into()
	};
	if tb::is_follower(task) {
		return with_button(
			"Repeats with its time block, on the block's days and at its time. Change it from the time block.",
			content.into(),
			change("Stop from this day", Action::StopRepeating(task.id.clone())),
		);
	}
	if !tb::is_assigned(task) {
		return content.into();
	}
	let note = "Scheduled by its time block. Change it from the time block.";
	let hour = tb::day_start_hour(state);
	match tb::block_of(task, &state.calendar.events, hour)
		.filter(|block| tb::can_repeat_with(task, block, hour))
	{
		Some(block) => with_button(
			note,
			content.into(),
			change(
				"Repeat from this day",
				Action::RepeatWithBlock(task.id.clone(), block.id.clone()),
			),
		),
		None => super::controls::owned_elsewhere(note, content, metrics.label()),
	}
}

/// The habit behind a planned item, with its usual daily tracking dialog.
pub(super) fn habit_link<'a>(
	state: &'a State,
	task: &Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let Some(habit) = state
		.habits
		.habit_of(task)
		.and_then(|id| state.habits.library.habit(id))
	else {
		return iced::widget::Space::new().into();
	};
	column![
		field_label("Habit", metrics),
		text(&habit.name).size(metrics.label()),
		button(text("Record habit").size(metrics.label()))
			.style(super::controls::secondary)
			.on_press(Message::Habits(crate::habits::Action::OpenDay(
				habit.id.clone(),
				task.start.date_naive(),
			))),
	]
	.spacing(6)
	.width(Fill)
	.into()
}

fn organization_fields<'a>(
	state: &'a State,
	task: &'a Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));
	let mut estimate =
		button(text(format!("{}m", tb::metadata(task).minutes)).size(metrics.label()))
			.style(super::controls::secondary)
			.width(Fill);
	if !busy {
		estimate = estimate.on_press(msg(crate::task_editor::Action::EditEstimate(
			task.id.clone(),
		)));
	}
	column![
		project_field(state, task, metrics),
		labels_field(state, task, metrics),
		location_field(state, task, metrics),
		column![field_label("Estimate", metrics), estimate].spacing(6),
		priority_field(state, task, metrics),
	]
	.spacing(12)
	.into()
}

/// When the task is due, in the same fields an event's times use. Times only
/// mean something once there is a day to put them on.
fn due_fields<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
	let busy = schedule_busy(state, task);
	let schedule = state.task_editor();
	let time_format = state.config.appearance.time_format;
	let size = metrics.label();
	let dated = !schedule.date.trim().is_empty();
	let mut body = column![field_label("Due Date", metrics)]
		.spacing(6)
		.width(Fill);
	if dated {
		let mut all_day = crate::ui::controls::checkbox(task.all_day)
			.label("All day")
			.text_size(size);
		if !busy {
			let id = task.id.clone();
			all_day = all_day.on_toggle(move |value| {
				msg(crate::task_editor::Action::DueAllDay(id.clone(), value))
			});
		}
		body = body.push(all_day);
	}
	let id = task.id.clone();
	body = body.push(super::controls::date(
		time_format,
		&schedule.date,
		true,
		false,
		!busy,
		size,
		move |value| msg(crate::task_editor::Action::DueDate(id.clone(), value)),
	));
	if dated && !task.all_day {
		let (start, end) = (task.id.clone(), task.id.clone());
		body = body
			.push(field_label("Start Time", metrics))
			.push(super::controls::time(
				time_format,
				&schedule.start_time,
				!busy,
				size,
				move |value| msg(crate::task_editor::Action::DueStart(start.clone(), value)),
			))
			.push(field_label("End Time", metrics))
			.push(super::controls::time(
				time_format,
				&schedule.end_time,
				!busy,
				size,
				move |value| msg(crate::task_editor::Action::DueEnd(end.clone(), value)),
			));
	}
	body.into()
}

/// The day a task has to be done by, with an optional time.
fn deadline_field<'a>(
	state: &'a State,
	task: &'a Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let id = task.id.clone();
	column![
		field_label("Deadline", metrics),
		super::controls::date(
			state.config.appearance.time_format,
			&state.task_editor().deadline,
			true,
			true,
			!schedule_busy(state, task),
			metrics.label(),
			move |value| msg(crate::task_editor::Action::Deadline(id.clone(), value)),
		),
	]
	.spacing(6)
	.width(Fill)
	.into()
}

fn schedule_busy(state: &State, task: &Event) -> bool {
	state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving))
}

fn field_label<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
	text(label)
		.size(metrics.label())
		.font(iced::Font {
			weight: iced::font::Weight::Bold,
			..iced::Font::DEFAULT
		})
		.style(text::secondary)
		.into()
}

/// The task's projects, plus a searchable picker for adding more.
fn project_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	let projects = tb::metadata(task).projects;
	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));

	let mut selected = column![].spacing(4).width(Fill);
	// A project that has since been removed is simply left out.
	for project in projects
		.iter()
		.filter_map(|id| state.config.projects.get(*id))
	{
		let mut remove = button(text("×").size(metrics.label()))
			.padding([0, 4])
			.style(crate::ui::controls::ghost);
		if !busy {
			remove = remove.on_press(msg(crate::task_editor::Action::ProjectRemove(
				task.id.clone(),
				project.id,
			)));
		}
		selected = selected.push(
			row![
				super::calendar::color_circle(
					project
						.color()
						.unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e))
				),
				text(&project.name).size(metrics.label()).width(Fill),
				remove,
			]
			.spacing(6)
			.align_y(iced::Center),
		);
	}
	if projects.is_empty() {
		selected = selected.push(
			text("No projects")
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		);
	}

	let open = state.task_editor().project_picker.as_deref() == Some(task.id.as_str());
	let mut toggle = button(
		text(if open { "Done" } else { "+ Add project" })
			.size(metrics.label())
			.center()
			.width(Fill),
	)
	.width(Fill)
	.style(crate::ui::controls::secondary);
	if !busy {
		toggle = toggle.on_press(msg(crate::task_editor::Action::ProjectPicker(
			task.id.clone(),
			!open,
		)));
	}

	let mut content = column![field_label("Projects", metrics), selected, toggle].spacing(6);

	if open {
		content = content.push(
			crate::ui::controls::text_input("Search projects…", &state.task_editor().project_query)
				.on_input(|query| msg(crate::task_editor::Action::ProjectQuery(query)))
				.size(metrics.label())
				.width(Fill),
		);

		let query = state.task_editor().project_query.trim().to_lowercase();
		let available: Vec<_> = state
			.config
			.projects
			.sorted()
			.into_iter()
			.filter(|project| !projects.contains(&project.id))
			.filter(|project| query.is_empty() || project.name.to_lowercase().contains(&query))
			.collect();

		if available.is_empty() {
			content = content.push(
				text(if query.is_empty() {
					"Every project is already added"
				} else {
					"No matching projects"
				})
				.size(metrics.label() - 1.0)
				.style(text::secondary),
			);
		}

		for project in available {
			let mut add = button(text(&project.name).size(metrics.label()).width(Fill))
				.width(Fill)
				.style(crate::ui::controls::ghost);
			if !busy {
				add = add.on_press(msg(crate::task_editor::Action::ProjectAdd(
					task.id.clone(),
					project.id,
				)));
			}
			content = content.push(
				row![
					super::calendar::color_circle(
						project
							.color()
							.unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e))
					),
					add,
				]
				.spacing(6)
				.align_y(iced::Center),
			);
		}
	}

	content.into()
}

/// Free-form labels for the task: the ones it has, a field to add another, and
/// matching labels already used elsewhere as suggestions.
fn labels_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	let labels = tb::metadata(task).labels;
	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));

	let mut content = column![field_label("Labels", metrics)]
		.spacing(4)
		.width(Fill);

	for label in &labels {
		let mut remove = button(text("×").size(metrics.label()))
			.padding([0, 4])
			.style(crate::ui::controls::ghost);
		if !busy {
			remove = remove.on_press(msg(crate::task_editor::Action::RemoveLabel(
				task.id.clone(),
				label.clone(),
			)));
		}
		content = content.push(
			row![
				text(label.clone()).size(metrics.label()).width(Fill),
				remove,
			]
			.spacing(6)
			.align_y(iced::Center),
		);
	}

	let mut input =
		crate::ui::controls::text_input("Add a label…", &state.task_editor().label_draft)
			.size(metrics.label())
			.width(Fill);
	let mut add = button(text("Add").size(metrics.label())).style(crate::ui::controls::primary);
	if !busy {
		let typed = state.task_editor().label_draft.clone();
		input = input
			.on_input(|value| msg(crate::task_editor::Action::LabelDraft(value)))
			.on_submit(msg(crate::task_editor::Action::AddLabel(
				task.id.clone(),
				typed.clone(),
			)));
		add = add.on_press(msg(crate::task_editor::Action::AddLabel(
			task.id.clone(),
			typed,
		)));
	}
	content = content.push(row![input, add].spacing(6).align_y(iced::Center));

	if let Some(error) = &state.task_editor().label_error {
		content = content.push(text(error).size(metrics.label()).style(text::danger));
	}

	// Labels already used on other tasks make spelling them again unnecessary.
	let query = state.task_editor().label_draft.trim().to_lowercase();
	if !query.is_empty() {
		let mut seen: Vec<String> = Vec::new();
		for other in state.calendar.events.iter().filter(|event| event.is_task()) {
			for label in tb::metadata(other).labels {
				if labels
					.iter()
					.any(|existing| existing.eq_ignore_ascii_case(&label))
					|| !label.to_lowercase().contains(&query)
					|| seen
						.iter()
						.any(|existing: &String| existing.eq_ignore_ascii_case(&label))
				{
					continue;
				}
				seen.push(label);
			}
		}
		seen.truncate(5);
		for label in seen {
			content = content.push(
				button(text(format!("+ {label}")).size(metrics.label() - 1.0))
					.width(Fill)
					.style(crate::ui::controls::ghost)
					.on_press(msg(crate::task_editor::Action::AddLabel(
						task.id.clone(),
						label,
					))),
			);
		}
	}

	content.into()
}

/// The task's priority: none, P1 or P2.
fn priority_field<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	let current = tb::metadata(task).priority;
	let busy = state.task_operations.busy.contains_task(&task.id)
		|| (task.id == crate::inbox::DRAFT_ID
			&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));

	let choices = std::iter::once((None, "None")).chain(
		tb::Priority::ALL
			.into_iter()
			.map(|priority| (Some(priority), priority.label())),
	);

	let mut options = row![].spacing(6);
	for (priority, label) in choices {
		let mut choice = button(text(label).size(metrics.label()).center().width(Fill))
			.width(Fill)
			.style(if priority == current {
				crate::ui::controls::primary
			} else {
				crate::ui::controls::secondary
			});
		if !busy {
			choice = choice.on_press(msg(crate::task_editor::Action::Priority(
				task.id.clone(),
				priority,
			)));
		}
		options = options.push(choice);
	}

	column![field_label("Priority", metrics), options]
		.spacing(4)
		.into()
}

/// Where the task happens: the place it was given, or a way to search for one.
fn location_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	let current = tb::metadata(task)
		.location
		.and_then(|id| state.config.travel.place(id));

	column![
		field_label("Location", metrics),
		super::travel::location_control(
			state,
			crate::travel::PickTarget::Task(task.id.clone()),
			current,
			"Add location",
			metrics,
		),
	]
	.spacing(4)
	.into()
}

/// Keep task actions in the same order in details and the backlog editor.
pub fn task_actions<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	let status = tb::task_status(task);
	let mut actions = column![].spacing(metrics.spacing).width(Fill);
	if crate::inbox::untriaged(state, task) {
		let mut triage = button(text("Triage").size(metrics.label()).center().width(Fill))
			.width(Fill)
			.style(crate::ui::controls::primary);
		if state.task_operations.busy.is_empty() {
			triage = triage.on_press(msg(Action::Triage(task.id.clone())));
		}
		actions = actions.push(triage);
	}
	for (label, action) in [
		(
			if task.is_habit() {
				if status == tb::TaskStatus::Completed {
					"Mark as not done"
				} else {
					"Mark as done"
				}
			} else if status == tb::TaskStatus::Completed {
				"Uncomplete task"
			} else {
				"Complete task"
			},
			Message::Task(TaskMessage::TaskToggled(task.id.clone())),
		),
		(
			if status == tb::TaskStatus::Ignored {
				"Unignore Task"
			} else {
				"Ignore Task"
			},
			Message::Task(TaskMessage::TaskIgnoreToggled(task.id.clone())),
		),
		("Move to inbox", msg(Action::MoveToInbox(task.id.clone()))),
		(
			"Move to backlog",
			msg(Action::BacklogPicker(task.id.clone())),
		),
		(
			"Delete task",
			Message::Task(TaskMessage::TaskDeleteRequested(task.id.clone())),
		),
	]
	.into_iter()
	.take(if task.is_habit() && task.is_calendar_task() {
		1
	} else {
		usize::MAX
	}) {
		let mut action_button = button(text(label).size(metrics.label()).center().width(Fill))
			.width(Fill)
			.style(
				if matches!(&action, Message::Task(TaskMessage::TaskToggled(_))) {
					crate::ui::controls::primary
				} else {
					crate::ui::controls::secondary
				},
			);
		if state.task_operations.busy.is_empty() {
			action_button = action_button.on_press(action);
		}
		actions = actions.push(action_button);
		if label == "Move to backlog"
			&& state.task_editor().backlog_picker.as_deref() == Some(task.id.as_str())
		{
			let mut choices = column![].spacing(4).width(Fill);
			for section in tb::BacklogSection::ALL {
				let mut choice = button(text(section.label()).size(metrics.label()))
					.width(Fill)
					.style(crate::ui::controls::secondary);
				if state.task_operations.busy.is_empty() {
					choice = choice.on_press(msg(Action::MoveToBacklog(task.id.clone(), section)));
				}
				choices = choices.push(choice);
			}
			actions = actions.push(container(choices).padding([0, 8]));
		}
	}
	actions.into()
}

pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let Some(block) = state
		.timeblocking
		.block
		.as_deref()
		.and_then(|id| state.calendar.event(id))
	else {
		return column![
			text("Time block unavailable"),
			crate::ui::controls::quiet_button("Close").on_press(msg(Action::Close))
		]
		.padding(16)
		.into();
	};
	let color_input = state
		.task_editor()
		.color_input
		.as_ref()
		.filter(|(id, _)| id == &block.id)
		.map(|(_, v)| v.as_str())
		.or_else(|| {
			state
				.calendar
				.edit_draft
				.as_ref()
				.filter(|d| d.id == block.id)
				.map(|d| d.color_input.as_str())
		})
		.or(block.details.color_id.as_deref())
		.unwrap_or("");
	let color_picker: Element<'a, Message> = if block.details.sending
		|| state
			.calendar
			.edit_draft
			.as_ref()
			.is_some_and(|d| d.id == block.id && d.saving)
	{
		text("Saving color…").size(metrics.label()).into()
	} else {
		crate::ui::calendar::item_color_picker(
			color_input,
			metrics,
			move |v| msg(crate::task_editor::Action::ColorInput(block.id.clone(), v)),
			move |v| msg(crate::task_editor::Action::SaveColor(block.id.clone(), v)),
		)
	};

	let tasks = tb::assigned(state, block);
	let capacity = (block.end - block.start).num_minutes().max(0) as u32;
	let block_id = block.id.clone();
	let compact_weekly = state.weekly() && metrics.size.width < 240.0;
	let delete = button(text("Delete").size(metrics.label()).center().width(Fill))
		.width(Fill)
		.on_press(Message::Event(EventMessage::EventDeleteRequested(
			block_id.clone(),
		)))
		.style(crate::ui::controls::secondary);
	let close = button(text("Close").size(metrics.label()).center().width(Fill))
		.width(Fill)
		.on_press(msg(Action::Close))
		.style(crate::ui::controls::secondary);
	let actions: Element<'a, Message> = if compact_weekly {
		column![delete, close].spacing(metrics.spacing).into()
	} else {
		row![delete, close].spacing(metrics.spacing).into()
	};
	let mut body = column![
		super::calendar::titled(state, block, text(block.title.clone()).size(18), metrics),
		text({
			let (start, end) = crate::calendar::time_format::range_parts(
				block.start,
				block.end,
				state.config.appearance.time_format,
			);
			format!("{} · {start}–{end}", block.start.format("%a, %b %-d"))
		})
		.size(metrics.label())
		.style(text::secondary),
		actions,
		color_picker,
		super::controls::section_heading("Planned Tasks", Some(tasks.len()), 16.0),
	]
	.spacing(10);
	if tb::block_repeats(block) {
		body = body.push(repeat_choice(state, metrics));
	}
	body = body.push(planned_slot(state, &block_id, metrics));
	if let Some(target) = crate::notes::target_of(state, block) {
		body = body.push(super::notes::section(state, target, metrics));
	}
	let mut elapsed = 0;
	for task in tasks {
		elapsed += tb::metadata(task).minutes;
		let mut remove = crate::ui::controls::quiet_button("−");
		if state.task_operations.busy.is_empty() {
			remove = remove.on_press(msg(Action::Unassign(task.id.clone())));
		}
		let card: Element<'a, Message> = if compact_weekly {
			column![
				super::calendar::weekly_task_chip(state, task, metrics),
				remove
			]
			.into()
		} else {
			row![task_row(state, task, metrics, false), remove].into()
		};
		let card: Element<'a, Message> = if tb::is_follower(task) {
			column![card, repeating_note(state, task, metrics)]
				.spacing(2)
				.into()
		} else if tb::can_repeat_with(task, block, tb::day_start_hour(state)) {
			column![card, one_off_note(state, task, block, metrics)]
				.spacing(2)
				.into()
		} else {
			card
		};
		body = body.push(if elapsed > capacity {
			container(card)
				.style(|_| container::Style {
					background: Some(Color::from_rgba(0.9, 0.2, 0.15, 0.22).into()),
					..Default::default()
				})
				.into()
		} else {
			card
		});
	}
	body = body.push(super::controls::section_heading(
		"Available tasks",
		None,
		16.0,
	));
	body = body.push(
		row![
			text("All tasks").size(metrics.label()).width(Fill),
			crate::ui::controls::toggler(state.timeblocking.available_all)
				.on_toggle(|all| msg(Action::AvailableAll(all))),
		]
		.spacing(8)
		.align_y(iced::Center),
	);
	body = body.push(
		row![
			text("Show planned tasks").size(metrics.label()).width(Fill),
			crate::ui::controls::toggler(state.timeblocking.available_planned)
				.on_toggle(|show| msg(Action::AvailablePlanned(show))),
		]
		.spacing(8)
		.align_y(iced::Center),
	);
	body = body.push(
		crate::ui::controls::text_input(
			"Search available tasks…",
			&state.timeblocking.available_query,
		)
		.on_input(|value| msg(Action::AvailableQuery(value)))
		.size(metrics.label())
		.width(Fill),
	);
	let query = state.timeblocking.available_query.trim().to_lowercase();
	let all_tasks = state.timeblocking.available_all;
	let hour = tb::day_start_hour(state);
	let available: Vec<_> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			offers_available(
				event,
				tb::in_block(event, block, hour),
				&query,
				all_tasks,
				state.timeblocking.available_planned,
			)
		})
		.collect();
	if available.is_empty() && !query.is_empty() {
		body = body.push(text("No matching tasks").style(text::secondary));
	} else if available.is_empty() && !all_tasks {
		body = body.push(text("No tasks for today").style(text::secondary));
	}
	for task in available {
		let add = crate::ui::controls::quiet_button("+")
			.on_press(msg(Action::Assign(task.id.clone(), block_id.clone())));
		if compact_weekly {
			body = body.push(column![
				super::calendar::weekly_task_chip(state, task, metrics),
				add
			]);
		} else {
			body = body.push(row![task_row(state, task, metrics, false), add]);
		}
	}
	body = body.push(available_habits(state, block, metrics));
	// A release anywhere on the panel assigns the dragged task, including on the
	// plus box. A release on a task row is captured first and reorders instead.
	mouse_area(
		container(crate::ui::controls::scrollable(body))
			.padding(16)
			.width(Fill)
			.height(Fill),
	)
	.on_release(msg(Action::DropBlock(block_id)))
	.into()
}

/// The library is independent of calendar occurrences and Habits tab filters.
fn available_habits<'a>(
	state: &'a State,
	block: &Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let query = state
		.timeblocking
		.available_habit_query
		.trim()
		.to_lowercase();
	let planned = tb::assigned_all(state, block);
	let mut body = column![
		super::controls::section_heading("Available habits", None, 16.0),
		crate::ui::controls::text_input(
			"Search all habits…",
			&state.timeblocking.available_habit_query
		)
		.on_input(|value| msg(Action::AvailableHabitQuery(value)))
		.size(metrics.label())
		.width(Fill),
	]
	.spacing(8);
	let mut count = 0;
	for habit in &state.habits.library.habits {
		if !habit.name.to_lowercase().contains(&query)
			|| planned
				.iter()
				.any(|task| tb::metadata(task).habit_id.as_deref() == Some(&habit.id))
		{
			continue;
		}
		count += 1;
		let swatch = container(iced::widget::Space::new().width(8).height(8)).style(
			move |theme: &iced::Theme| container::Style {
				background: Some(
					timeparagon_core::habits::palette_color(
						habit.color,
						theme.extended_palette().is_dark,
					)
					.into(),
				),
				border: iced::Border {
					radius: 4.0.into(),
					..Default::default()
				},
				..Default::default()
			},
		);
		let mut add = crate::ui::controls::quiet_button("+");
		if state.task_operations.busy.is_empty() {
			add = add.on_press(msg(Action::AddHabit(habit.id.clone(), block.id.clone())));
		}
		body = body.push(
			row![
				swatch,
				text(&habit.name).size(metrics.label()).width(Fill),
				text(if habit.archived {
					"Habit · Archived"
				} else {
					"Habit"
				})
				.size(metrics.label() - 2.0)
				.style(text::secondary),
				add,
			]
			.spacing(8)
			.align_y(iced::Center),
		);
	}
	if count == 0 {
		body = body.push(
			text(if query.is_empty() {
				"No available habits"
			} else {
				"No matching habits"
			})
			.size(metrics.label())
			.style(text::secondary),
		);
	}
	body.into()
}

/// Whether what is added to a repeating block repeats with it. An item in
/// a time block either repeats with the block or is a one-off.
fn repeat_choice<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let repeat = !state.timeblocking.one_off;
	column![
		row![
			text("Repeat with this block")
				.size(metrics.label())
				.width(Fill),
			crate::ui::controls::toggler(repeat).on_toggle(|repeat| msg(Action::OneOff(!repeat))),
		]
		.spacing(8)
		.align_y(iced::Center),
		text(if repeat {
			"Tasks added here come back every time the block does."
		} else {
			"Tasks added here are for this day only."
		})
		.size(metrics.label() - 1.0)
		.style(text::secondary),
	]
	.spacing(2)
	.into()
}

/// Under a task repeating with its block: says so, and offers to stop it
/// from this occurrence on.
fn repeating_note<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
	item_note(
		state,
		"↻ Repeats with this block",
		"Stop from this day",
		Action::StopRepeating(task.id.clone()),
		metrics,
	)
}

/// Under a one-off in a repeating block: says so, and offers to have it
/// repeat with the block from this occurrence on.
fn one_off_note<'a>(
	state: &'a State,
	task: &Event,
	block: &Event,
	metrics: &Metrics,
) -> Element<'a, Message> {
	item_note(
		state,
		"This day only",
		"Repeat from this day",
		Action::RepeatWithBlock(task.id.clone(), block.id.clone()),
		metrics,
	)
}

/// A line under a planned item: how it repeats, and a button to change it.
fn item_note<'a>(
	state: &'a State,
	label: &'a str,
	action_label: &'a str,
	action: Action,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let mut change = button(text(action_label).size(metrics.label() - 1.0))
		.padding([2, 6])
		.style(crate::ui::controls::ghost);
	if state.task_operations.busy.is_empty() {
		change = change.on_press(msg(action));
	}
	row![
		text(label)
			.size(metrics.label() - 1.0)
			.style(text::secondary)
			.width(Fill),
		change,
	]
	.spacing(6)
	.align_y(iced::Center)
	.into()
}

/// The planned-tasks box. The plus sits in the middle until it is clicked,
/// which opens a field that creates a Google Task in this block. Dropping a
/// task on the box assigns it without opening the field.
fn planned_slot<'a>(state: &'a State, block_id: &str, metrics: &Metrics) -> Element<'a, Message> {
	let adding = state.task_operations.busy.adding_todo();
	let open = state.timeblocking.todo_open || adding || state.timeblocking.todo_error.is_some();
	let dragging = state.timeblocking.dragging.is_some();
	let plus = container(text("+").size(28)).center_x(Fill);
	if !open {
		return button(plus.center_y(Length::Fixed(72.0)))
			.padding(0)
			.width(Fill)
			.on_press(msg(Action::OpenTodo))
			.style(move |theme: &iced::Theme, status| add_slot_style(theme, status, dragging))
			.into();
	}
	container(column![plus, todo_editor(state, block_id, metrics)].spacing(8))
		.padding(10)
		.width(Fill)
		.style(move |theme: &iced::Theme| add_slot_container(theme, dragging))
		.into()
}

/// A free-text line that becomes a Google Task in this block.
fn todo_editor<'a>(state: &'a State, block_id: &str, metrics: &Metrics) -> Element<'a, Message> {
	let adding = state.task_operations.busy.adding_todo();
	let ready =
		state.task_operations.busy.is_empty() && !state.timeblocking.todo_draft.trim().is_empty();
	let mut input = crate::ui::controls::text_input("Write a task", &state.timeblocking.todo_draft)
		.id(iced::widget::Id::new("time-block-todo"))
		.size(metrics.label())
		.width(Fill);
	let mut add = crate::ui::controls::action_button(
		text(if adding { "Adding…" } else { "Add task" }).size(metrics.label()),
	);
	let mut cancel = button(text("Cancel").size(metrics.label())).style(crate::ui::controls::ghost);
	if !adding {
		input = input.on_input(|value| msg(Action::TodoDraft(value)));
		cancel = cancel.on_press(msg(Action::CloseTodo));
	}
	if ready {
		let id = block_id.to_owned();
		input = input.on_submit(msg(Action::AddTodo(id.clone())));
		add = add.on_press(msg(Action::AddTodo(id)));
	}
	let mut body = column![input, row![add, cancel].spacing(8)].spacing(6);
	if let Some(error) = &state.timeblocking.todo_error {
		body = body.push(text(error).size(metrics.label()).style(text::danger));
	}
	body.into()
}

fn add_slot_style(theme: &iced::Theme, status: button::Status, dragging: bool) -> button::Style {
	let palette = theme.extended_palette();
	let hot = dragging || matches!(status, button::Status::Hovered | button::Status::Pressed);
	button::Style {
		background: Some(
			if hot {
				palette.primary.weak.color
			} else {
				palette.background.weakest.color
			}
			.into(),
		),
		text_color: palette.primary.base.color,
		border: iced::Border {
			color: if hot {
				palette.primary.strong.color
			} else {
				palette.background.strong.color
			},
			width: if dragging { 2.0 } else { 1.0 },
			radius: crate::ui::controls::RADIUS_ROW.into(),
		},
		shadow: crate::ui::controls::shadow(theme, 0.1, 1.0, 3.0),
		..Default::default()
	}
}

fn add_slot_container(theme: &iced::Theme, dragging: bool) -> container::Style {
	let palette = theme.extended_palette();
	container::Style {
		background: Some(
			if dragging {
				palette.primary.weak.color
			} else {
				palette.background.weakest.color
			}
			.into(),
		),
		border: iced::Border {
			color: if dragging {
				palette.primary.strong.color
			} else {
				palette.background.strong.color
			},
			width: if dragging { 2.0 } else { 1.0 },
			radius: crate::ui::controls::RADIUS_ROW.into(),
		},
		..Default::default()
	}
}
/// The Backlog tab: only the Google Tasks on the chosen backlog list, so a
/// task is here because it was deliberately parked, not merely because it is
/// unscheduled.
pub fn backlog<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let list_id = state
		.config
		.calendar
		.backlog_list
		.clone()
		.unwrap_or_else(|| {
			format!(
				"tasks:{}",
				crate::config::CalendarSettings::DEFAULT_BACKLOG_LIST
			)
		});
	let list_name = tb::task_list_name(state, &list_id);

	let mut tasks: Vec<_> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			event.is_task()
				&& !event.completed
				&& !tb::is_assigned(event)
				&& tb::is_backlog_list(state, &event.calendar_id)
		})
		.collect();
	tasks.sort_by_key(|event| tb::backlog_order(event));

	let query = state.timeblocking.backlog_query.trim().to_lowercase();
	let shown: Vec<_> = tasks
		.iter()
		.copied()
		.filter(|task| query.is_empty() || task.title.to_lowercase().contains(&query))
		.collect();

	let mut body = column![
		super::controls::page_heading(
			list_name.clone(),
			"Google Tasks parked on this list",
			Some(shown.len()),
			metrics
		),
		text("Search").size(metrics.label()).style(text::secondary),
		crate::ui::controls::text_input("Search tasks…", &state.timeblocking.backlog_query)
			.on_input(|value| msg(Action::BacklogQuery(value)))
			.size(metrics.label())
			.width(Fill),
	]
	.spacing(8);
	if tasks.is_empty() {
		body = body.push(text(format!("No tasks on the {list_name} list")).style(text::secondary));
	} else if shown.is_empty() {
		body = body.push(text("No matching tasks").style(text::secondary));
	}
	for section in tb::BacklogSection::ALL {
		let mut contents = column![text(section.label()).size(metrics.label())].spacing(6);
		for task in shown
			.iter()
			.copied()
			.filter(|task| tb::metadata(task).backlog_section == section)
		{
			contents = contents.push(task_row(state, task, metrics, true));
		}
		let dragging = state.timeblocking.dragging.is_some();
		let section_card =
			container(contents)
				.padding(8)
				.width(Fill)
				.style(move |theme: &iced::Theme| {
					let palette = theme.extended_palette();
					container::Style {
						border: iced::Border {
							color: if dragging {
								palette.primary.base.color
							} else {
								palette.background.strong.color
							},
							width: 1.0,
							radius: crate::ui::controls::RADIUS_CARD.into(),
						},
						..crate::ui::controls::card_surface(theme)
					}
				});
		body = body
			.push(mouse_area(section_card).on_release(msg(Action::DropBacklogSection(section))));
	}
	let content: Element<'a, Message> = container(crate::ui::controls::scrollable(body))
		.padding(metrics.padding)
		.width(Fill)
		.height(Fill)
		.into();
	content
}

pub use super::timeblocking_focus::distraction_overlay;
pub use super::timeblocking_focus::session_editor;
pub use super::timeblocking_focus::view as focus_overlay;

/// Every estimate entry point opens the same app-wide dialog.
pub(super) fn estimate_button<'a>(
	state: &State,
	task: &Event,
	label_size: f32,
) -> Element<'a, Message> {
	let minutes = tb::metadata(task).minutes;
	let mut control = button(text(format!("Estimate: {minutes}m")).size(label_size))
		.style(crate::ui::controls::ghost);
	if !state.task_operations.busy.contains_task(&task.id) {
		control = control.on_press(msg(crate::task_editor::Action::EditEstimate(
			task.id.clone(),
		)));
	}
	control.into()
}

pub fn estimate_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	use iced::widget::{Space, stack};
	let task = state
		.task_editor()
		.estimate_task
		.as_deref()
		.and_then(|id| tb::event(state, id));
	let mut body = column![text("Estimated completion time").size(18)].spacing(10);
	if let Some(task) = task {
		let busy = state.task_operations.busy.contains_task(&task.id)
			|| (task.id == crate::inbox::DRAFT_ID
				&& state.inbox.draft.as_ref().is_some_and(|d| d.saving));
		let mut input = crate::ui::controls::text_input("Minutes", &state.task_editor().estimate)
			.id(iced::widget::Id::new("task-estimate"))
			.size(metrics.label());
		let mut save =
			crate::ui::controls::action_button(text(if busy { "Saving…" } else { "Save" }));
		if !busy {
			input = input
				.on_input(|value| msg(crate::task_editor::Action::Estimate(value)))
				.on_submit(msg(crate::task_editor::Action::SaveEstimate(
					task.id.clone(),
				)));
			save = save.on_press(msg(crate::task_editor::Action::SaveEstimate(
				task.id.clone(),
			)));
		}
		body = body
			.push(text(task.title.clone()).size(metrics.label()))
			.push(
				text("Estimate in minutes (0 means no estimate)")
					.size(metrics.label())
					.style(text::secondary),
			)
			.push(input);
		if let Some(error) = &state.task_editor().estimate_error {
			body = body.push(text(error).size(metrics.label()).style(text::danger));
		}
		body = body.push(
			row![
				save,
				button("Cancel")
					.on_press(msg(crate::task_editor::Action::CloseEstimate))
					.style(crate::ui::controls::ghost)
			]
			.spacing(8),
		);
	} else {
		body = body.push(text("This task is no longer available")).push(
			crate::ui::controls::quiet_button("Close")
				.on_press(msg(crate::task_editor::Action::CloseEstimate)),
		);
	}
	let card = container(crate::ui::controls::scrollable(body))
		.padding(16)
		.width(Fill)
		.max_width(420)
		.max_height((metrics.size.height - 32.0).max(80.0))
		.style(crate::ui::controls::surface);
	let backdrop =
		mouse_area(
			container(Space::new())
				.width(Fill)
				.height(Fill)
				.style(|_| container::Style {
					background: Some(crate::ui::controls::SCRIM.into()),
					..Default::default()
				}),
		)
		.on_press(msg(crate::task_editor::Action::CloseEstimate))
		.on_scroll(|_| Message::Ignored);
	let foreground = container(
		mouse_area(card)
			.on_press(Message::Ignored)
			.on_scroll(|_| Message::Ignored),
	)
	.padding(16)
	.center_x(Fill)
	.center_y(Fill);
	stack![backdrop, foreground].into()
}

/// Asks whether a recurring task or habit moves into the block alone, or
/// with its future occurrences deleted.
pub fn recurring_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let Some(pending) = state.timeblocking.pending_recurring.as_ref() else {
		return column![].into();
	};
	let (kind, name) = match &pending.item {
		tb::RecurringItem::Task(id) => (
			"task",
			state
				.calendar
				.event(id)
				.map(|task| task.title.clone())
				.unwrap_or_default(),
		),
		tb::RecurringItem::Habit(id) => (
			"habit",
			state
				.habits
				.library
				.habit(id)
				.map(|habit| habit.name.clone())
				.unwrap_or_default(),
		),
	};
	let block_repeats = state
		.calendar
		.event(&pending.block)
		.is_some_and(tb::block_repeats);
	let detail = if block_repeats {
		format!(
			"“{name}” repeats. Put just this day's {kind} in the time block, or have it repeat with the time block from now on. Repeating with the block replaces this occurrence and the later ones."
		)
	} else {
		format!(
			"“{name}” repeats. Move just this occurrence into the time block and keep the rest, or move it and delete the later occurrences."
		)
	};
	let busy = !state.task_operations.busy.is_empty();
	let size = metrics.label();
	let choice = |label: String, future: bool| {
		button(text(label).size(size).center().width(Fill))
			.width(Fill)
			.style(super::controls::secondary)
			.on_press_maybe((!busy).then_some(msg(Action::MoveRecurring(future))))
	};
	let body = column![
		text(format!("Move recurring {kind}?")).size(20),
		text(detail).size(size),
		choice("Just this day".to_owned(), false),
		choice(
			if block_repeats {
				"Repeat with this block from now".to_owned()
			} else {
				format!("Move this {kind} and delete later occurrences")
			},
			true
		),
		button(text("Cancel").size(size).center().width(Fill))
			.width(Fill)
			.style(super::controls::ghost)
			.on_press(msg(Action::CancelRecurring)),
	]
	.spacing(12);
	modal(body, msg(Action::CancelRecurring))
}

/// Asks before a one-off habit repeats with its block, since that ends the
/// habit's own repeating event from this day.
pub fn join_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let Some(pending) = state.timeblocking.pending_join.as_ref() else {
		return column![].into();
	};
	let name = state
		.calendar
		.event(&pending.task)
		.map(|task| task.title.clone())
		.unwrap_or_default();
	let busy = !state.task_operations.busy.is_empty();
	let size = metrics.label();
	let body = column![
        text("Repeat with this block?").size(20),
        text(format!(
            "“{name}” also has a repeating event of its own. Repeating with the time block ends that one from this day on, so the habit is not on the calendar twice."
        ))
        .size(size),
        button(text("Repeat with this block").size(size).center().width(Fill))
            .width(Fill)
            .style(super::controls::secondary)
            .on_press_maybe((!busy).then_some(msg(Action::ConfirmRepeatWithBlock))),
        button(text("Cancel").size(size).center().width(Fill))
            .width(Fill)
            .style(super::controls::ghost)
            .on_press(msg(Action::CancelRepeatWithBlock)),
    ]
    .spacing(12);
	modal(body, msg(Action::CancelRepeatWithBlock))
}

/// A dialog over a dimmed backdrop; clicking the backdrop sends `cancel`.
fn modal<'a>(body: impl Into<Element<'a, Message>>, cancel: Message) -> Element<'a, Message> {
	use iced::widget::{Space, stack};
	let backdrop =
		mouse_area(
			container(Space::new())
				.width(Fill)
				.height(Fill)
				.style(|_| container::Style {
					background: Some(super::controls::SCRIM.into()),
					..Default::default()
				}),
		)
		.on_press(cancel);
	let card = container(
		mouse_area(
			container(body)
				.padding(20)
				.max_width(460)
				.style(super::controls::surface),
		)
		.on_press(Message::Ignored),
	)
	.padding(16)
	.center_x(Fill)
	.center_y(Fill);
	stack![backdrop, card].into()
}

/// Whether a block's "Available tasks" list offers `event`, which is in
/// that block already when `here`.
///
/// With "All tasks" on, every open task is offered: any task list, dated or
/// not, timed on the calendar or planned in another block, whatever "Show
/// planned tasks" says. Only completed and ignored tasks, and those already
/// planned in this block, are left out. Otherwise only today's tasks are
/// offered, and planned ones only when "Show planned tasks" is on.
fn offers_available(
	event: &Event,
	here: bool,
	query: &str,
	all_tasks: bool,
	show_planned: bool,
) -> bool {
	let meta = tb::metadata(event);
	if !event.is_task() || event.completed || meta.ignored || here {
		return false;
	}
	if !query.is_empty() && !event.title.to_lowercase().contains(query) {
		return false;
	}
	if all_tasks {
		return true;
	}
	!tb::is_assigned(event)
		&& (show_planned || (event.all_day && tb::in_backlog(event)))
		&& (all_tasks || tb::on_today(event))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar;
	use iced::{Point, Size, mouse};

	#[test]
	fn the_recurring_prompt_offers_both_moves_and_cancel() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("read", "Read a book"));
		state.timeblocking.pending_recurring = Some(tb::PendingRecurring {
			item: tb::RecurringItem::Habit("read".into()),
			block: "standup".into(),
		});
		for (label, expected) in [
			("Just this day", Some(false)),
			("Move this habit and delete later occurrences", Some(true)),
			("Cancel", None),
		] {
			let size = Size::new(360.0, 900.0);
			let metrics = Metrics::of(size);
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				size,
				recurring_confirmation(&state, &metrics),
			);
			assert!(ui.find("Move recurring habit?").is_ok());
			let _ = ui.click(label).unwrap();
			let message = ui.into_messages().next();
			match expected {
				Some(future) => assert!(matches!(
					message,
					Some(Message::TimeBlocking(Action::MoveRecurring(f))) if f == future
				)),
				None => assert!(matches!(
					message,
					Some(Message::TimeBlocking(Action::CancelRecurring))
				)),
			}
		}
	}

	#[test]
	fn a_repeating_block_asks_whether_new_tasks_repeat_and_marks_those_that_do() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let block = state.calendar.event("standup").unwrap().clone();
		state.config.calendar.app_calendar = Some(block.calendar_id.clone());
		let size = Size::new(360.0, 1200.0);
		let metrics = Metrics::of(size);
		state.timeblocking.block = Some("standup".into());
		{
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				size,
				panel(&state, &metrics),
			);
			assert!(ui.find("Repeat with this block").is_err());
		}
		state
			.calendar
			.event_mut("standup")
			.unwrap()
			.details
			.recurring_event_id = Some("standup-series".into());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "deep_1".into();
		task.calendar_id = block.calendar_id.clone();
		task.details.recurring_event_id = Some("deep".into());
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 100,
			metadata: tb::Metadata {
				block_series: Some(format!("{}/standup-series", block.calendar_id)),
				minutes: 30,
				..Default::default()
			},
		});
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			panel(&state, &metrics),
		);
		assert!(ui.find("Repeat with this block").is_ok());
		assert!(ui.find("↻ Repeats with this block").is_ok());
		let _ = ui.click("Stop from this day").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			message,
			Message::TimeBlocking(Action::StopRepeating(id)) if id == "deep_1"
		)));

		// A one-off in it offers to repeat with it instead.
		let key = tb::block_key(state.calendar.event("standup").unwrap());
		let task = state.calendar.event_mut("deep_1").unwrap();
		task.details.recurring_event_id = None;
		let meta = &mut task.details.task_data.as_mut().unwrap().metadata;
		meta.block_series = None;
		meta.block = Some(key);
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			panel(&state, &metrics),
		);
		assert!(ui.find("↻ Repeats with this block").is_err());
		assert!(ui.find("This day only").is_ok());
		let _ = ui.click("Repeat from this day").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			message,
			Message::TimeBlocking(Action::RepeatWithBlock(task, block))
				if task == "deep_1" && block == "standup"
		)));
	}

	#[test]
	fn repeating_a_habit_with_its_block_is_confirmed_or_cancelled() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		state.timeblocking.pending_join = Some(tb::PendingJoin {
			task: "deep".into(),
			block: "standup".into(),
		});
		for (label, confirm) in [("Repeat with this block", true), ("Cancel", false)] {
			let size = Size::new(360.0, 900.0);
			let metrics = Metrics::of(size);
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				size,
				join_confirmation(&state, &metrics),
			);
			assert!(ui.find("Repeat with this block?").is_ok());
			let _ = ui.click(label).unwrap();
			let message = ui.into_messages().next();
			assert!(match message {
				Some(Message::TimeBlocking(Action::ConfirmRepeatWithBlock)) => confirm,
				Some(Message::TimeBlocking(Action::CancelRepeatWithBlock)) => !confirm,
				_ => false,
			});
		}
	}

	#[test]
	fn a_task_in_a_block_shows_its_scheduling_grayed_out() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 100,
			metadata: tb::Metadata {
				minutes: 30,
				..Default::default()
			},
		});
		state.task_editor_mut().date = "2026-10-29".into();
		let reason = "Scheduled by its time block. Change it from the time block.";
		let size = Size::new(450.0, 1400.0);
		let metrics = Metrics::of(size);
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			fields(&state, task, &metrics),
		);
		assert!(ui.find(reason).is_err());
		let _ = ui.click("Oct 29, 2026").unwrap();
		assert!(ui.into_messages().next().is_some());

		let task = state.calendar.event_mut("task:deep").unwrap();
		task.details.task_data.as_mut().unwrap().metadata.block = Some("work/standup".into());
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			fields(&state, task, &metrics),
		);
		assert!(ui.find(reason).is_ok());
		assert!(ui.find("Repeat from this day").is_err());
		let _ = ui.click("Oct 29, 2026");
		assert!(
			ui.into_messages()
				.all(|message| matches!(message, Message::Ignored))
		);

		// A calendar task in a repeating block can start repeating with it.
		let block = state.calendar.event_mut("standup").unwrap();
		block.details.recurring_event_id = Some("standup-series".into());
		let (key, calendar_id) = (tb::block_key(block), block.calendar_id.clone());
		let task = state.calendar.event_mut("task:deep").unwrap();
		task.id = "deep".into();
		task.calendar_id = calendar_id;
		task.details.task_data.as_mut().unwrap().metadata.block = Some(key);
		let task = state.calendar.event("deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			fields(&state, task, &metrics),
		);
		assert!(ui.find(reason).is_ok());
		let _ = ui.click("Repeat from this day").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			message,
			Message::TimeBlocking(Action::RepeatWithBlock(task, block))
				if task == "deep" && block == "standup"
		)));
	}

	#[test]
	fn available_habits_browses_library_and_adds_selected_habit() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		// A library habit is available without any calendar series, even if
		// hidden by the Habits tab's filters.
		state.habits.library.hide_completed = true;
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("read", "Read a book"));
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("walk", "Take a walk"));
		let size = Size::new(450.0, 900.0);
		let metrics = Metrics::of(size);
		let block = state.calendar.event("standup").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			available_habits(&state, block, &metrics),
		);
		assert!(ui.find("Available habits").is_ok());
		assert!(ui.find("Read a book").is_ok());
		assert!(ui.find("Take a walk").is_ok());
		drop(ui);
		state.timeblocking.available_habit_query = "  BOOK  ".into();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			available_habits(&state, state.calendar.event("standup").unwrap(), &metrics),
		);
		assert!(ui.find("Take a walk").is_err());
		ui.click("+").unwrap();
		assert!(ui.into_messages().any(|message| matches!(message, Message::TimeBlocking(Action::AddHabit(id, block)) if id == "read" && block == "standup")));
		let block_key = tb::block_key(state.calendar.event("standup").unwrap());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:read".into();
		task.calendar_id = "tasks:list".into();
		task.details.task_data = Some(timeparagon_core::task_db::TaskData {
			id: 1,
			metadata: tb::Metadata {
				habit_id: Some("read".into()),
				block: Some(block_key),
				..Default::default()
			},
		});
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			available_habits(&state, state.calendar.event("standup").unwrap(), &metrics),
		);
		assert!(ui.find("Read a book").is_err());
		assert!(ui.find("No matching habits").is_ok());
	}

	#[test]
	fn all_tasks_offers_every_open_task_but_not_completed_ignored_or_already_here() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		task.title = "hh".into();
		task.all_day = false;
		let task = state.calendar.event("task:deep").unwrap().clone();
		let offers = |event: &Event, query| {
			let here = tb::metadata(event).block.as_deref() == Some("cal/here");
			offers_available(event, here, query, true, false)
		};
		assert!(offers(&task, ""));
		assert!(offers(&task, "hh"));
		assert!(!offers(&task, "zz"));

		let with_block = |block: &str| {
			let mut planned = task.clone();
			let mut meta = tb::metadata(&planned);
			meta.block = Some(block.into());
			planned.details.task_data = Some(timeparagon_core::task_db::TaskData {
				id: 1,
				metadata: meta,
			});
			planned
		};
		// Planned in another block, or against a block that no longer
		// resolves (such as a recurring series rather than one occurrence).
		assert!(offers(&with_block("cal/series"), "hh"));
		assert!(!offers(&with_block("cal/here"), "hh"));

		let mut done = task.clone();
		done.completed = true;
		assert!(!offers(&done, "hh"));
		let mut ignored = task.clone();
		let mut meta = tb::metadata(&ignored);
		meta.ignored = true;
		ignored.details.task_data = Some(timeparagon_core::task_db::TaskData {
			id: 1,
			metadata: meta,
		});
		assert!(!offers(&ignored, "hh"));
	}

	#[test]
	fn inbox_button_moves_the_selected_task() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		let size = Size::new(320.0, 900.0);
		let metrics = Metrics::of(size);
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			task_actions(&state, state.calendar.event("task:deep").unwrap(), &metrics),
		);
		ui.click("Move to inbox").unwrap();
		assert!(ui.into_messages().any(|message| matches!(message,
            Message::TimeBlocking(Action::MoveToInbox(id)) if id == "task:deep")));
	}

	#[test]
	fn backlog_picker_offers_every_group_and_selects_the_task_destination() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		let size = Size::new(320.0, 900.0);
		let metrics = Metrics::of(size);
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			size,
			task_actions(&state, state.calendar.event("task:deep").unwrap(), &metrics),
		);
		assert!(ui.find("Next week or two").is_err());
		ui.click("Move to backlog").unwrap();
		let message = ui.into_messages().next().unwrap();
		assert!(
			matches!(&message, Message::TimeBlocking(Action::BacklogPicker(id)) if id == "task:deep")
		);
		let _ = crate::app::update(&mut state, message);
		assert!(state.task_operations.busy.is_empty());
		for section in tb::BacklogSection::ALL {
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				size,
				task_actions(&state, state.calendar.event("task:deep").unwrap(), &metrics),
			);
			for group in tb::BacklogSection::ALL {
				assert!(ui.find(group.label()).is_ok(), "{}", group.label());
			}
			ui.click(section.label()).unwrap();
			assert!(ui.into_messages().any(|message| matches!(message,
                Message::TimeBlocking(Action::MoveToBacklog(id, choice))
                    if id == "task:deep" && choice == section)));
		}
		let _ = tb::update(&mut state, Action::BacklogPicker("task:deep".into()));
		assert!(state.task_editor().backlog_picker.is_none());
	}

	#[test]
	fn task_title_click_starts_inline_editing() {
		let mut state = State::blank();
		state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		let metrics = Metrics::of(Size::new(450.0, 900.0));
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			Size::new(450.0, 900.0),
			editable_title(&state, task, &metrics),
		);
		ui.click(task.title.as_str()).unwrap();
		assert!(ui.into_messages().any(|m| matches!(
			m,
			Message::TaskEditor(crate::task_editor::Action::EditTitle)
		)));
	}

	fn dated_task_state() -> State {
		let mut state = State::blank();
		state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		state.task_editor_mut().date = "2026-10-29".into();
		state.task_editor_mut().deadline = "2026-10-30".into();
		state.task_editor_mut().start_time = "09:00".into();
		state.task_editor_mut().end_time = "09:30".into();
		state
	}

	#[test]
	fn a_task_is_scheduled_with_regular_date_and_time_fields() {
		let mut state = dated_task_state();
		state.calendar.event_mut("task:deep").unwrap().all_day = false;
		let metrics = Metrics::of(Size::new(450.0, 900.0));
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			Size::new(450.0, 900.0),
			due_fields(&state, task, &metrics),
		);
		for label in ["Due Date", "All day", "Start Time", "End Time"] {
			assert!(ui.find(label).is_ok(), "{label}");
		}
		for gone in ["Schedule Day", "Schedule Time", "Reset Time", "Save time"] {
			assert!(ui.find(gone).is_err(), "{gone}");
		}
		ui.click("All day").unwrap();
		assert!(ui.into_messages().any(|m| matches!(
			m,
			Message::TaskEditor(crate::task_editor::Action::DueAllDay(id, true)) if id == "task:deep"
		)));

		// A task for the whole day has no clock times to set.
		state.calendar.event_mut("task:deep").unwrap().all_day = true;
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			Size::new(450.0, 900.0),
			due_fields(&state, task, &metrics),
		);
		assert!(ui.find("Start Time").is_err());
		drop(ui);

		// Nor does one without a day.
		state.task_editor_mut().date.clear();
		let task = state.calendar.event("task:deep").unwrap();
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			Size::new(450.0, 900.0),
			due_fields(&state, task, &metrics),
		);
		assert!(ui.find("All day").is_err());
		assert!(ui.find("Choose date").is_ok());
	}

	#[test]
	fn both_date_fields_can_be_cleared_and_only_the_deadline_picks_a_time() {
		let state = dated_task_state();
		let metrics = Metrics::of(Size::new(450.0, 900.0));
		let task = state.calendar.event("task:deep").unwrap();
		for (field, day, with_time) in [
			(due_fields(&state, task, &metrics), "Oct 29, 2026", false),
			(deadline_field(&state, task, &metrics), "Oct 30, 2026", true),
		] {
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				Size::new(450.0, 900.0),
				field,
			);
			ui.click(day).unwrap();
			assert!(ui.into_messages().any(|m| matches!(
				m,
				Message::Picker(super::super::controls::Action::Open(p))
					if p.optional && p.with_time == with_time
			)));
		}
	}

	#[test]
	fn estimate_popup_covers_the_time_block_view_at_narrow_and_wide_widths() {
		for width in [200.0, 800.0] {
			let mut state = State::blank();
			state.calendar =
				crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
			let task = state.calendar.event_mut("deep").unwrap();
			task.id = "task:deep".into();
			task.calendar_id = "tasks:list".into();
			state.timeblocking.focus = Some("standup".into());
			let _ = tb::update(&mut state, Action::EditEstimate("task:deep".into()));
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				Size::new(width, 600.0),
				super::super::view(&state),
			);
			assert!(ui.find("Save").is_ok());
			assert!(ui.find("Cancel").is_ok());
			ui.point_at(Point::new(2.0, 2.0));
			ui.simulate([
				iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
				iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
			]);
			let messages: Vec<_> = ui.into_messages().collect();
			assert!(messages.iter().any(|m| matches!(
				m,
				Message::TaskEditor(crate::task_editor::Action::CloseEstimate)
			)));
			assert!(
				!messages
					.iter()
					.any(|m| matches!(m, Message::TimeBlocking(Action::CloseView)))
			);
		}
	}
}
