//! The Routines screen: an overview of the day's, week's and month's
//! routines, and the routine in progress, one step at a time.
//!
//! Everything is a single column so it works in the narrow panel. Figures
//! follow the app's chart conventions: stat tiles for headline numbers, one
//! hue for magnitude, text in text colors, and a tooltip on every bar.

use chrono::{Duration, NaiveDate};
use iced::widget::{Space, button, column, container, row, text, tooltip};
use iced::{Alignment, Color, Element, Fill, FillPortion, Length, Padding, Theme};

use crate::app::{EventMessage, Message, State};
use crate::calendar::Event;
use crate::routines::{
	self as r, Action, Change, DeadlinePolicy, Decision, Flow, Group, Period, Routine, Scale,
	Section, Step,
};
use crate::timeblocking::{self as tb, BacklogSection, Priority};

use super::controls;
use super::layout::Metrics;

fn msg(action: Action) -> Message {
	Message::Routines(action)
}

/// How many tasks a list shows before "Show all".
const PREVIEW: usize = 4;
/// The estimates offered as one-press choices.
const ESTIMATES: [u32; 6] = [15, 30, 45, 60, 90, 120];
/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 12.0;

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let body = match &state.routines.flow {
		Some(flow) => flow_view(state, flow, metrics),
		None => overview(state, metrics),
	};
	container(body)
		.padding(metrics.padding)
		.width(Fill)
		.height(Fill)
		.into()
}

fn bold() -> iced::Font {
	iced::Font {
		weight: iced::font::Weight::Semibold,
		..iced::Font::DEFAULT
	}
}

fn muted<'a>(content: impl text::IntoFragment<'a>, metrics: &Metrics) -> text::Text<'a, Theme> {
	text(content)
		.size(metrics.label() - 1.0)
		.style(text::secondary)
		.wrapping(text::Wrapping::WordOrGlyph)
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
	container(content)
		.padding([8, 10])
		.width(Fill)
		.style(controls::card_surface)
		.into()
}

fn heading<'a>(
	label: impl Into<String>,
	count: Option<usize>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	controls::section_heading(label, count, metrics.label() + 2.0).into()
}

/// A small choice button. Without a message it is drawn disabled, unless it
/// is the current choice, which is drawn filled.
fn chip<'a>(
	label: impl Into<String>,
	message: Option<Message>,
	selected: bool,
	metrics: &Metrics,
) -> Element<'a, Message> {
	if selected && message.is_none() {
		return container(text(label.into()).size(metrics.label() - 1.0))
			.padding([3, 8])
			.style(|theme: &Theme| {
				let primary = theme.extended_palette().primary.base;
				container::Style {
					background: Some(primary.color.into()),
					text_color: Some(primary.text),
					border: iced::Border {
						radius: controls::RADIUS_CONTROL.into(),
						..Default::default()
					},
					..Default::default()
				}
			})
			.into();
	}
	button(text(label.into()).size(metrics.label() - 1.0))
		.padding([3, 8])
		.style(if selected {
			controls::primary
		} else {
			controls::secondary
		})
		.on_press_maybe(message)
		.into()
}

fn chips<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
	row(items).spacing(4).wrap().vertical_spacing(4.0).into()
}

/// One headline number: what it is, the figure, and a line of context.
fn stat<'a>(
	label: &'a str,
	value: String,
	detail: String,
	metrics: &Metrics,
) -> Element<'a, Message> {
	container(
		column![
			muted(label, metrics),
			text(value).size(metrics.label() + 7.0).font(bold()),
			muted(detail, metrics),
		]
		.spacing(1),
	)
	.padding([6, 8])
	.width(Fill)
	.style(controls::card_surface)
	.into()
}

/// Headline numbers, two to a row when the panel is narrow.
fn stats<'a>(tiles: Vec<Element<'a, Message>>, metrics: &Metrics) -> Element<'a, Message> {
	let per_row = if metrics.size.width < 360.0 { 2 } else { 4 };
	let mut rows = column![].spacing(6).width(Fill);
	let mut tiles = tiles.into_iter().peekable();
	while tiles.peek().is_some() {
		let mut line = row![].spacing(6).width(Fill);
		for _ in 0..per_row {
			match tiles.next() {
				Some(tile) => line = line.push(tile),
				None => line = line.push(Space::new().width(Fill)),
			}
		}
		rows = rows.push(line);
	}
	rows.into()
}

/// A ratio against a limit: the fill on a lighter track of the same hue,
/// turning to the danger hue once it goes over.
fn meter<'a>(value: u32, limit: u32) -> Element<'a, Message> {
	let over = value > limit;
	let (filled, empty) = if limit == 0 {
		(u16::from(value > 0), u16::from(value == 0))
	} else if over {
		(1, 0)
	} else {
		let share = (f64::from(value) / f64::from(limit) * 1000.0).round() as u16;
		(share, 1000 - share)
	};
	let fill = container(Space::new())
		.width(FillPortion(filled.max(1)))
		.height(8)
		.style(move |theme: &Theme| {
			let palette = theme.extended_palette();
			container::Style {
				background: Some(
					if over {
						palette.danger.base.color
					} else {
						palette.primary.base.color
					}
					.into(),
				),
				border: iced::Border {
					radius: 4.0.into(),
					..Default::default()
				},
				..Default::default()
			}
		});
	let mut bar = row![].width(Fill);
	if filled > 0 {
		bar = bar.push(fill);
	}
	if empty > 0 {
		bar = bar.push(Space::new().width(FillPortion(empty)));
	}
	container(bar)
		.width(Fill)
		.height(8)
		.style(move |theme: &Theme| {
			let palette = theme.extended_palette();
			container::Style {
				background: Some(
					if over {
						palette.danger.weak.color
					} else {
						palette.primary.weak.color
					}
					.into(),
				),
				border: iced::Border {
					radius: 4.0.into(),
					..Default::default()
				},
				..Default::default()
			}
		})
		.into()
}

/// Columns of one series on one baseline, each with its value on the cap,
/// its label under it and the details in a tooltip.
fn columns<'a>(values: Vec<(String, u32, String)>, metrics: &Metrics) -> Element<'a, Message> {
	const HEIGHT: f32 = 64.0;
	let max = values.iter().map(|(_, v, _)| *v).max().unwrap_or(0).max(1);
	let mut bars = row![].spacing(2).width(Fill).align_y(Alignment::End);
	for (label, value, details) in values {
		let height = (value as f32 / max as f32 * HEIGHT).max(if value > 0 { 3.0 } else { 0.0 });
		let bar = container(Space::new())
			.width(Length::Fixed(18.0))
			.height(height)
			.style(|theme: &Theme| container::Style {
				background: Some(theme.extended_palette().primary.base.color.into()),
				border: iced::Border {
					radius: iced::border::Radius {
						top_left: 4.0,
						top_right: 4.0,
						bottom_right: 0.0,
						bottom_left: 0.0,
					},
					..Default::default()
				},
				..Default::default()
			});
		let cap = text(if value > 0 {
			value.to_string()
		} else {
			String::new()
		})
		.size(metrics.label() - 2.0)
		.style(text::secondary);
		let column = column![
			cap,
			bar,
			container(Space::new())
				.width(Fill)
				.height(1)
				.style(|theme: &Theme| container::Style {
					background: Some(controls::edge_color(theme).into()),
					..Default::default()
				}),
			text(label)
				.size(metrics.label() - 2.0)
				.style(text::secondary),
		]
		.spacing(2)
		.width(Fill)
		.align_x(Alignment::Center);
		bars = bars.push(
			tooltip(
				container(column).width(Fill),
				container(text(details).size(metrics.label() - 1.0)).padding([4, 8]),
				tooltip::Position::Top,
			)
			.style(controls::surface),
		);
	}
	container(bars).width(Fill).into()
}

/// A name, a bar and a figure per row, the bars on one scale, largest first.
fn ranked<'a>(
	rows: Vec<(Option<Color>, String, u64, String)>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let max = rows.iter().map(|(_, _, v, _)| *v).max().unwrap_or(0).max(1);
	let mut list = column![].spacing(6).width(Fill);
	for (swatch, name, value, figure) in rows {
		let share = ((value as f64 / max as f64) * 1000.0).round() as u16;
		let mut label = row![].spacing(6).align_y(Alignment::Center);
		if let Some(color) = swatch {
			label = label.push(super::calendar::color_circle(color));
		}
		label = label
			.push(
				text(name)
					.size(metrics.label() - 1.0)
					.width(Fill)
					.wrapping(text::Wrapping::WordOrGlyph),
			)
			.push(
				text(figure)
					.size(metrics.label() - 1.0)
					.style(text::secondary),
			);
		let mut bar = row![
			container(Space::new())
				.width(FillPortion(share.max(1)))
				.height(6)
				.style(|theme: &Theme| container::Style {
					background: Some(theme.extended_palette().primary.base.color.into()),
					border: iced::Border {
						radius: 3.0.into(),
						..Default::default()
					},
					..Default::default()
				})
		]
		.width(Fill);
		if share < 1000 {
			bar = bar.push(Space::new().width(FillPortion(1000 - share)));
		}
		list = list.push(column![label, bar].spacing(3));
	}
	list.into()
}

/// "1 task", "3 tasks".
fn count(n: usize, one: &str, many: &str) -> String {
	format!("{n} {}", if n == 1 { one } else { many })
}

fn project_name(state: &State, id: Option<u64>) -> (Option<Color>, String) {
	match id.and_then(|id| state.config.projects.items.iter().find(|p| p.id == id)) {
		Some(project) => (project.color(), project.name.clone()),
		None => (None, "No project".into()),
	}
}

fn short_day(day: NaiveDate) -> String {
	day.format("%a, %b %-d").to_string()
}

/// What a task row says under its title.
fn task_meta(state: &State, task: &Event) -> String {
	let metadata = tb::metadata(task);
	let mut parts = Vec::new();
	parts.push(if metadata.minutes > 0 {
		r::duration(metadata.minutes)
	} else {
		"No estimate".into()
	});
	if let Some(priority) = metadata.priority {
		parts.push(priority.label().into());
	}
	if let Some(project) = metadata.projects.first() {
		parts.push(project_name(state, Some(*project)).1);
	}
	let hour = state.config.calendar.day_start_hour();
	if let Some(day) = r::task_day(task, hour) {
		if task.all_day {
			parts.push(short_day(day));
		} else {
			parts.push(format!(
				"{} {}",
				short_day(day),
				crate::calendar::time_format::clock(
					task.start,
					state.config.appearance.time_format
				)
			));
		}
	} else {
		parts.push(tb::task_list_name(state, &task.calendar_id));
	}
	if let Some(deadline) = metadata.deadline {
		let passed = deadline.is_past(crate::clock::now().naive_local());
		parts.push(if passed {
			format!("Deadline passed {}", short_day(deadline.date))
		} else {
			format!("Due {}", short_day(deadline.date))
		});
	}
	parts.join(" · ")
}

/// Whether a change to the task is on its way.
fn pending(state: &State, task: &Event) -> bool {
	state.routines.queued(&tb::task_key(task))
		|| state.timeblocking.busy.contains_task(&task.id)
		|| task.details.sending
}

/// A task with its details, opening the task's own editor when pressed, and
/// the given actions under it. While a change to it is on its way the
/// actions are replaced by a note saying so.
fn task_row<'a>(
	state: &'a State,
	task: &'a Event,
	actions: Vec<Element<'a, Message>>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let pending = pending(state, task);
	let title = button(
		text(&task.title)
			.size(metrics.label())
			.font(controls::pending_font(bold(), pending))
			.wrapping(text::Wrapping::WordOrGlyph)
			.width(Fill),
	)
	.padding(0)
	.width(Fill)
	.style(controls::ghost)
	.on_press(Message::Event(EventMessage::EventOpened(task.id.clone())));
	let mut content = column![title, muted(task_meta(state, task), metrics)].spacing(2);
	if pending {
		content = content.push(muted("Saving…", metrics));
	} else if !actions.is_empty() {
		content = content.push(chips(actions));
	}
	let shown = state.calendar.open_event.as_deref() == Some(task.id.as_str());
	let row = container(content)
		.padding([6, 8])
		.width(Fill)
		.style(move |theme: &Theme| {
			let palette = theme.extended_palette();
			container::Style {
				border: iced::Border {
					color: if shown {
						palette.primary.base.color
					} else {
						palette.background.strong.color
					},
					width: 1.0,
					radius: controls::RADIUS_ROW.into(),
				},
				..controls::row_surface(theme)
			}
		});
	if state.day_plan_active() && !pending {
		super::task_drag::task_drag(row, &task.id, state.timeblocking.dragging.as_deref())
	} else {
		row.into()
	}
}

fn change(task: &Event, change: Change) -> Option<Message> {
	Some(msg(Action::Task(tb::task_key(task), change)))
}

/// A titled list of tasks offered for picking, showing a few until opened.
fn group<'a>(
	state: &'a State,
	group: Group,
	title: &'a str,
	tasks: Vec<&'a Event>,
	actions: impl Fn(&'a Event) -> Vec<Element<'a, Message>>,
	metrics: &Metrics,
) -> Option<Element<'a, Message>> {
	if tasks.is_empty() {
		return None;
	}
	let open = state.routines.expanded.contains(&group);
	let count = tasks.len();
	let mut list = column![heading(title, Some(count), metrics)].spacing(6);
	for task in tasks.iter().take(if open { usize::MAX } else { PREVIEW }) {
		list = list.push(task_row(state, task, actions(task), metrics));
	}
	if count > PREVIEW {
		list = list.push(
			button(
				text(if open {
					"Show fewer".to_owned()
				} else {
					format!("Show all {count}")
				})
				.size(metrics.label() - 1.0),
			)
			.padding([2, 0])
			.style(controls::ghost)
			.on_press(msg(Action::Expand(group))),
		);
	}
	Some(list.into())
}

// ---------------------------------------------------------------------------
// The overview.

fn overview<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let today = r::today(state);
	let anchor = r::anchor(state);
	let mut page = column![].spacing(metrics.spacing + 4.0).width(Fill);

	let mut navigation = row![
		button(text("◀").size(metrics.label()))
			.padding([2, 8])
			.style(controls::ghost)
			.on_press(msg(Action::Shift(-1))),
		text(if anchor == today {
			"Today".to_owned()
		} else {
			anchor.format("%a, %b %-d, %Y").to_string()
		})
		.size(metrics.label())
		.width(Fill)
		.center(),
		button(text("▶").size(metrics.label()))
			.padding([2, 8])
			.style(controls::ghost)
			.on_press(msg(Action::Shift(1))),
	]
	.spacing(4)
	.align_y(Alignment::Center);
	if anchor != today {
		navigation = navigation.push(chip("Today", Some(msg(Action::Shift(0))), false, metrics));
	}
	page = page.push(controls::page_heading(
		"Routines",
		"Plan, refresh and reflect, one step at a time",
		None,
		metrics,
	));
	page = page.push(navigation);

	if let Some(paused) = &state.routines.paused {
		let steps = paused.routine.steps();
		page = page.push(
			container(
				column![
					text(format!(
						"{} · {}",
						paused.routine.title(),
						paused.period.label()
					))
					.size(metrics.label())
					.font(bold()),
					muted(
						format!(
							"In progress: step {} of {}, {}",
							paused.step + 1,
							steps.len(),
							paused.current().title(paused.period.scale).to_lowercase()
						),
						metrics
					),
					row![
						chip("Continue", Some(msg(Action::Resume)), true, metrics),
						chip("Discard", Some(msg(Action::Discard)), false, metrics),
					]
					.spacing(4),
				]
				.spacing(4),
			)
			.padding([8, 10])
			.width(Fill)
			.style(controls::summary_surface),
		);
	}

	let suggested = (anchor == today).then(|| r::suggested(state)).flatten();
	for scale in Scale::ALL {
		let period = r::period(state, scale);
		let current = period.contains(today);
		let title = match (scale, current) {
			(Scale::Day, true) => format!("Today · {}", period.label()),
			(Scale::Week, true) => format!("This week · {}", period.label()),
			(Scale::Month, true) => format!("This month · {}", period.label()),
			_ => period.label(),
		};
		let mut section = column![heading(title, None, metrics)].spacing(6);
		for routine in Routine::ALL.into_iter().filter(|r| r.scale() == scale) {
			section = section.push(routine_card(
				state,
				routine,
				period,
				suggested == Some(routine),
				metrics,
			));
		}
		page = page.push(section);
	}

	controls::scrollable(container(page).padding(Padding {
		right: SCROLLBAR_GUTTER,
		..Padding::ZERO
	}))
	.height(Fill)
	.into()
}

/// The overview's line about a routine's period.
fn status(state: &State, routine: Routine, period: Period) -> String {
	let entry = r::entry(state, period);
	let format = state.config.appearance.time_format;
	let at = |time: chrono::DateTime<chrono::Utc>| {
		crate::calendar::time_format::clock(time.with_timezone(&chrono::Local), format)
	};
	match routine {
		Routine::DailyRefresh => {
			if period.first != r::today(state) {
				return entry.map_or_else(
					|| "Only for today".into(),
					|entry| match entry.refreshes.len() {
						0 => "Only for today".into(),
						1 => "Refreshed once".into(),
						n => format!("Refreshed {n} times"),
					},
				);
			}
			let counts: Vec<String> = r::refresh_lists(state)
				.into_iter()
				.filter(|(_, tasks)| !tasks.is_empty())
				.map(|(section, tasks)| {
					format!("{} {}", tasks.len(), section.label().to_lowercase())
				})
				.collect();
			let waiting = if counts.is_empty() {
				"Nothing waiting".to_owned()
			} else {
				counts.join(" · ")
			};
			match entry.and_then(|entry| entry.refreshes.last()) {
				Some(last) => format!("{waiting} · last at {}", at(last.recorded_at)),
				None => waiting,
			}
		}
		_ => {
			let Some(entry) = entry.filter(|_| routine.done(entry)) else {
				return match routine {
					Routine::DailyPlan => {
						let tasks = r::day_tasks(state, period.first);
						if tasks.is_empty() {
							"Not planned yet".into()
						} else {
							format!(
								"Not planned yet · {} on the day",
								count(tasks.len(), "task", "tasks")
							)
						}
					}
					_ if routine.plans() => "Not planned yet".into(),
					_ => "Not reflected on yet".into(),
				};
			};
			let mut parts = Vec::new();
			if routine.plans() {
				if let Some(time) = entry.planned_at {
					parts.push(format!("Planned at {}", at(time)));
				}
				if !entry.planned_tasks.is_empty() {
					parts.push(format!(
						"{}, {}",
						count(entry.planned_tasks.len(), "task", "tasks"),
						r::duration(entry.planned_minutes)
					));
				}
				if !entry.objectives.is_empty() {
					parts.push(format!("{} objectives", entry.objectives.len()));
				}
				if !entry.intention.trim().is_empty() {
					parts.push(format!("“{}”", entry.intention.trim()));
				}
			} else {
				parts.push("Reflected".into());
				if let Some(rating) = entry.rating {
					parts.push(format!("rated {rating}/5"));
				}
				if !entry.objectives.is_empty() {
					let met = entry.objectives.iter().filter(|o| o.done).count();
					parts.push(format!(
						"{met} of {} objectives met",
						entry.objectives.len()
					));
				}
			}
			parts.join(" · ")
		}
	}
}

fn routine_card<'a>(
	state: &'a State,
	routine: Routine,
	period: Period,
	suggested: bool,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let done = routine.done(r::entry(state, period));
	let today = r::today(state);
	let available = routine != Routine::DailyRefresh || period.first == today;
	let label = if !available {
		None
	} else if state
		.routines
		.paused
		.as_ref()
		.is_some_and(|flow| flow.routine == routine && flow.period == period)
	{
		Some("Continue")
	} else if done {
		Some(if routine == Routine::DailyRefresh {
			"Run again"
		} else {
			"Open"
		})
	} else {
		Some("Start")
	};
	let mut title = row![
		text(if done { "✓" } else { "○" })
			.size(metrics.label())
			.style(if done { text::success } else { text::secondary }),
		text(routine.title())
			.size(metrics.label() + 1.0)
			.font(bold())
			.width(Fill),
	]
	.spacing(6)
	.align_y(Alignment::Center);
	if suggested {
		title = title.push(
			container(text("Suggested").size(metrics.label() - 2.0))
				.padding([0, 7])
				.style(controls::tag),
		);
	}
	let mut content = column![title, muted(status(state, routine, period), metrics)].spacing(3);
	if metrics.size.width >= 300.0 {
		content = content.push(muted(routine.blurb(), metrics));
	}
	let action = label.map(|label| {
		button(text(label).size(metrics.label() - 1.0))
			.padding([3, 10])
			.style(if done {
				controls::secondary
			} else {
				controls::primary
			})
			.on_press(msg(Action::Start(routine)))
	});
	let body: Element<'a, Message> = match action {
		Some(action) if metrics.size.width >= 300.0 => row![content.width(Fill), action]
			.spacing(8)
			.align_y(Alignment::Center)
			.into(),
		Some(action) => column![content, action].spacing(6).into(),
		None => content.into(),
	};
	container(body)
		.padding([8, 10])
		.width(Fill)
		.style(if suggested {
			controls::summary_surface
		} else {
			controls::card_surface
		})
		.into()
}

// ---------------------------------------------------------------------------
// A routine in progress.

fn flow_view<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let scale = flow.period.scale;
	let steps = flow.routine.steps();
	let step = flow.current();

	let back = button(text("‹ Routines").size(metrics.label() - 1.0))
		.padding([2, 0])
		.style(controls::ghost)
		.on_press(msg(Action::Close));
	let title = column![
		back,
		text(flow.routine.title())
			.size(if metrics.breakpoint.is_compact() {
				16.0
			} else {
				20.0
			})
			.font(bold()),
		muted(flow.period.label(), metrics),
	]
	.spacing(1);

	// The steps, as tabs to jump between. The refresh's cards are reached in
	// order, so its steps are shown but not jumped to.
	let jumpable = flow.routine != Routine::DailyRefresh;
	let progress: Element<'a, Message> = if metrics.size.width >= 340.0 {
		chips(
			steps
				.iter()
				.enumerate()
				.map(|(index, s)| {
					chip(
						format!("{}. {}", index + 1, s.title(scale)),
						(jumpable && index != flow.step).then(|| msg(Action::Step(index))),
						index == flow.step,
						metrics,
					)
				})
				.collect(),
		)
	} else {
		muted(
			format!(
				"Step {} of {} · {}",
				flow.step + 1,
				steps.len(),
				step.title(scale)
			),
			metrics,
		)
		.into()
	};

	let mut top = column![title, progress].spacing(6).width(Fill);
	top = top.push(
		text(step.question(scale))
			.size(metrics.label() + 1.0)
			.wrapping(text::Wrapping::WordOrGlyph),
	);

	let content = match step {
		Step::Review => review(state, flow, metrics),
		Step::ChooseTasks => choose_tasks(state, flow, metrics),
		Step::Estimate => estimate(state, flow, metrics),
		Step::Timebox => timebox(state, flow, metrics),
		Step::Highlight | Step::Focus => focus(state, flow, metrics),
		Step::RefreshStart => refresh_start(state, flow, metrics),
		Step::Triage => triage(state, flow, metrics),
		Step::RefreshDone => refresh_done(state, flow, metrics),
		Step::Report => report(state, flow, metrics),
		Step::Unfinished => unfinished(state, flow, metrics),
		Step::ObjectivesReview => objectives_review(state, flow, metrics),
		Step::Reflect => reflect(state, flow, metrics),
		Step::Wrapup => wrapup(state, flow, metrics),
		Step::LookBack => look_back(state, flow, metrics),
		Step::Objectives => objectives(state, flow, metrics),
		Step::Schedule => schedule(state, flow, metrics),
	};
	let scroll = controls::scrollable(container(content).width(Fill).padding(Padding {
		right: SCROLLBAR_GUTTER,
		..Padding::ZERO
	}))
	.height(Fill);

	column![top, scroll, footer(state, flow, metrics)]
		.spacing(metrics.spacing)
		.height(Fill)
		.into()
}

fn footer<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let step = flow.current();
	let at_start = flow.step == 0 && !(step == Step::Triage && flow.refresh.index > 0);
	let back = button(text("Back").size(metrics.label()))
		.padding([4, 12])
		.style(controls::secondary)
		.on_press_maybe((!at_start).then(|| msg(Action::Back)));
	let next_label = if flow.last_step() {
		match (flow.routine, &flow.then) {
			(_, Some(_)) => "Continue planning",
			(Routine::DailyRefresh, None) => "Done",
			(routine, None) if routine.plans() => "Finish planning",
			_ => "Finish",
		}
	} else if step == Step::Triage {
		"Skip the rest"
	} else if step == Step::RefreshStart {
		"Start"
	} else {
		"Next"
	};
	let next = button(text(next_label).size(metrics.label()))
		.padding([4, 12])
		.style(controls::primary)
		.on_press(msg(Action::Next));
	let saving = state.routines.saving(!state.timeblocking.busy.is_empty());
	let note = if saving > 0 {
		format!("Saving {saving}…")
	} else if state.routines.loading {
		"Loading the calendar…".into()
	} else {
		String::new()
	};
	row![back, muted(note, metrics).width(Fill).center(), next]
		.spacing(8)
		.align_y(Alignment::Center)
		.into()
}

// ---------------------------------------------------------------------------
// Daily plan.

fn review<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let today = flow.period.first;
	let yesterday = today - Duration::days(1);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);

	let mut last = column![heading("Yesterday", None, metrics)].spacing(4);
	let done = r::done_on(state, yesterday).len();
	last = last.push(
		text(match done {
			0 => "Nothing was ticked off yesterday.".to_owned(),
			1 => "You finished 1 task yesterday.".to_owned(),
			n => format!("You finished {n} tasks yesterday."),
		})
		.size(metrics.label()),
	);
	if let Some(entry) = r::entry(state, Period::day(yesterday)) {
		if !entry.intention.trim().is_empty() {
			let outcome = match entry.intention_met {
				Some(true) => " · done",
				Some(false) => " · not done",
				None => "",
			};
			last = last.push(muted(
				format!("Highlight: {}{outcome}", entry.intention.trim()),
				metrics,
			));
		}
		let first = entry.answer(Routine::carried_prompt(Scale::Day)).trim();
		if !first.is_empty() {
			last =
				last.push(text(format!("You meant to start with: {first}")).size(metrics.label()));
		}
	}
	page = page.push(card(last));

	let week = Period::of(Scale::Week, today, state.config.calendar.first_day_of_week);
	let mut objectives = column![heading("This week's objectives", None, metrics)].spacing(4);
	match r::entry(state, week).filter(|entry| !entry.objectives.is_empty()) {
		Some(entry) => {
			for objective in &entry.objectives {
				objectives = objectives.push(
					text(format!(
						"{} {}",
						if objective.done { "✓" } else { "•" },
						objective.text
					))
					.size(metrics.label())
					.wrapping(text::Wrapping::WordOrGlyph),
				);
			}
		}
		None => {
			objectives = objectives.push(muted("None set yet.", metrics));
			objectives = objectives.push(chip(
				"Plan the week",
				Some(msg(Action::Start(Routine::WeeklyPlan))),
				false,
				metrics,
			));
		}
	}
	page = page.push(card(objectives));

	let lists = r::refresh_lists(state);
	let waiting: usize = lists.iter().map(|(_, tasks)| tasks.len()).sum();
	let mut sorting = column![heading("Waiting to be sorted", Some(waiting), metrics)].spacing(4);
	if waiting == 0 {
		sorting = sorting.push(muted(
			"All clear: nothing unfinished or in the inbox.",
			metrics,
		));
	} else {
		for (section, tasks) in &lists {
			if !tasks.is_empty() {
				sorting = sorting.push(
					text(format!(
						"{} {} · {}",
						tasks.len(),
						section.label().to_lowercase(),
						section.about().to_lowercase()
					))
					.size(metrics.label() - 1.0),
				);
			}
		}
		sorting = sorting.push(chip(
			"Run the daily refresh first",
			Some(msg(Action::Detour)),
			true,
			metrics,
		));
	}
	page = page.push(card(sorting));
	page.into()
}

fn choose_tasks<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let day = flow.period.first;
	let tomorrow = day + Duration::days(1);
	let tasks = r::day_tasks(state, day);
	let (minutes, _) = r::estimated(&tasks);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);

	// Planning another day, such as tomorrow, names it instead.
	let name = if day == r::today(state) {
		"Today".to_owned()
	} else {
		day.format("%A").to_string()
	};
	let mut list = column![heading(
		format!("{name} · {}", r::duration(minutes)),
		Some(tasks.len()),
		metrics
	)]
	.spacing(6);
	if tasks.is_empty() {
		list = list.push(muted(
			"Nothing planned yet. Add tasks below, or pull them in from the lists.",
			metrics,
		));
	}
	for task in &tasks {
		list = list.push(task_row(
			state,
			task,
			vec![
				chip(
					"Tomorrow",
					change(task, Change::Day(tomorrow)),
					false,
					metrics,
				),
				chip(
					"Backlog",
					change(task, Change::Backlog(BacklogSection::NextWeekOrTwo)),
					false,
					metrics,
				),
			],
			metrics,
		));
	}
	let done = r::done_on(state, day).len();
	if done > 0 {
		list = list.push(muted(format!("✓ {done} already done"), metrics));
	}
	page = page.push(list);

	let placeholder = format!("Add a task for {}", name.to_lowercase());
	let input = controls::text_input(&placeholder, &state.routines.quick_add)
		.on_input(|text| msg(Action::QuickAdd(text)))
		.on_submit(msg(Action::QuickAddSubmit))
		.size(metrics.label())
		.width(Fill);
	let add = button(
		text(if state.routines.adding {
			"Adding…"
		} else {
			"Add"
		})
		.size(metrics.label()),
	)
	.style(controls::primary)
	.on_press_maybe((!state.routines.adding).then(|| msg(Action::QuickAddSubmit)));
	page = page.push(row![input, add].spacing(6).align_y(Alignment::Center));

	let add_label = format!("+ {name}");
	let add_today = |task: &Event| {
		vec![chip(
			add_label.clone(),
			change(task, Change::Day(day)),
			true,
			metrics,
		)]
	};
	let not_today = |tasks: Vec<&'a Event>| -> Vec<&'a Event> {
		tasks
			.into_iter()
			.filter(|task| r::task_day(task, state.config.calendar.day_start_hour()) != Some(day))
			.collect()
	};
	let week = Period::of(Scale::Week, day, state.config.calendar.first_day_of_week);
	let later: Vec<&Event> = (1..=(week.last() - day).num_days())
		.flat_map(|offset| r::day_tasks(state, day + Duration::days(offset)))
		.collect();
	let groups = [
		group(
			state,
			Group::Unfinished,
			"Unfinished from earlier days",
			r::unfinished(state, day),
			add_today,
			metrics,
		),
		group(
			state,
			Group::DueSoon,
			"Due within a week",
			not_today(r::due_by(state, day + Duration::days(7))),
			add_today,
			metrics,
		),
		group(
			state,
			Group::LaterThisWeek,
			"Later this week",
			later,
			add_today,
			metrics,
		),
		group(
			state,
			Group::Triaged,
			"Triaged",
			not_today(crate::inbox::triaged_tasks(state)),
			add_today,
			metrics,
		),
		group(
			state,
			Group::NextWeeks,
			"Backlog · next week or two",
			r::backlog(state, BacklogSection::NextWeekOrTwo),
			add_today,
			metrics,
		),
		group(
			state,
			Group::Inbox,
			"Inbox",
			crate::inbox::tasks(state),
			add_today,
			metrics,
		),
	];
	let groups: Vec<_> = groups.into_iter().flatten().collect();
	if !groups.is_empty() {
		page = page.push(muted("Pull work in from your lists:", metrics));
		for group in groups {
			page = page.push(group);
		}
	}
	page.into()
}

/// The planned time against the free time of `day`, with what to make of it.
fn capacity<'a>(
	state: &'a State,
	day: NaiveDate,
	tasks: &[&Event],
	metrics: &Metrics,
) -> Element<'a, Message> {
	let (planned, missing) = r::estimated(tasks);
	let free = r::free_minutes(state, day);
	let mut content = column![
		row![
			text(format!("Planned {}", r::duration(planned)))
				.size(metrics.label() + 1.0)
				.font(bold())
				.width(Fill),
			text(format!("Free {}", r::duration(free)))
				.size(metrics.label())
				.style(text::secondary),
		]
		.align_y(Alignment::Center),
		meter(planned, free),
	]
	.spacing(6);
	let verdict = if planned > free {
		format!(
			"Over by {}. Move something to another day.",
			r::duration(planned - free)
		)
	} else if planned == 0 {
		"Give your tasks estimates to see whether the day is full.".into()
	} else {
		format!("Fits, with {} to spare.", r::duration(free - planned))
	};
	content = content.push(
		text(verdict)
			.size(metrics.label() - 1.0)
			.style(if planned > free {
				text::danger
			} else {
				text::secondary
			}),
	);
	if missing > 0 {
		content = content.push(muted(
			format!(
				"{missing} {} no estimate yet.",
				if missing == 1 {
					"task has"
				} else {
					"tasks have"
				}
			),
			metrics,
		));
	}
	content = content.push(muted(
		"Free time is your waking hours left today, less meetings and other events.",
		metrics,
	));
	card(content)
}

fn estimate<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let day = flow.period.first;
	let tasks = r::day_tasks(state, day);
	let over = r::estimated(&tasks).0 > r::free_minutes(state, day);
	let mut page = column![capacity(state, day, &tasks, metrics)]
		.spacing(metrics.spacing + 2.0)
		.width(Fill);
	if tasks.is_empty() {
		page = page.push(muted(
			"No tasks planned for today yet. Go back a step to add some.",
			metrics,
		));
	}
	for task in tasks {
		let metadata = tb::metadata(task);
		let mut actions: Vec<Element<'a, Message>> = ESTIMATES
			.iter()
			.map(|&minutes| {
				chip(
					r::duration(minutes),
					(metadata.minutes != minutes)
						.then(|| change(task, Change::Estimate(minutes)))
						.flatten(),
					metadata.minutes == minutes,
					metrics,
				)
			})
			.collect();
		let urgent = metadata.priority == Some(Priority::P1);
		actions.push(chip(
			"P1",
			change(task, Change::Priority((!urgent).then_some(Priority::P1))),
			urgent,
			metrics,
		));
		if over {
			actions.push(chip(
				"Tomorrow",
				change(task, Change::Day(day + Duration::days(1))),
				false,
				metrics,
			));
		}
		page = page.push(task_row(state, task, actions, metrics));
	}
	page.into()
}

fn timebox<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let day = flow.period.first;
	let blocks = r::blocks_on(state, day);
	let format = state.config.appearance.time_format;
	let clock = |time| crate::calendar::time_format::clock(time, format);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	page = page.push(muted(
		"Drag a task onto the day beside this panel to give it a time, or put it in a time block here.",
		metrics,
	));

	let mut list = column![heading("Time blocks", Some(blocks.len()), metrics)].spacing(6);
	if blocks.is_empty() {
		list = list.push(muted(
			"No time blocks today. Make some in the day view.",
			metrics,
		));
	}
	for block in &blocks {
		let assigned = tb::assigned(state, block);
		let used: u32 = assigned.iter().map(|task| tb::metadata(task).minutes).sum();
		let length = (block.end - block.start).num_minutes().max(0) as u32;
		let mut content = column![
			row![
				text(&block.title)
					.size(metrics.label())
					.font(bold())
					.width(Fill),
				muted(
					format!("{} – {}", clock(block.start), clock(block.end)),
					metrics
				),
			]
			.spacing(6),
			meter(used, length),
			muted(
				format!(
					"{} · {} of {}",
					count(assigned.len(), "task", "tasks"),
					r::duration(used),
					r::duration(length)
				),
				metrics
			),
		]
		.spacing(4);
		for task in assigned.iter().take(6) {
			content = content.push(text(format!("• {}", task.title)).size(metrics.label() - 1.0));
		}
		list = list.push(card(content));
	}
	page = page.push(list);

	let unslotted = crate::inbox::today_tasks(state);
	let mut waiting = column![heading("Without a time", Some(unslotted.len()), metrics)].spacing(6);
	if unslotted.is_empty() {
		waiting = waiting.push(muted(
			"Every task for today has a time or a block.",
			metrics,
		));
	}
	for task in unslotted {
		let actions = blocks
			.iter()
			.map(|block| {
				chip(
					format!("{} {}", block.title, clock(block.start)),
					change(task, Change::Block(block.id.clone())),
					false,
					metrics,
				)
			})
			.collect();
		waiting = waiting.push(task_row(state, task, actions, metrics));
	}
	page = page.push(waiting);
	page = page.push(chip(
		"Open the day planner",
		Some(msg(Action::OpenPlanner)),
		false,
		metrics,
	));
	page.into()
}

/// The plan's last step: its highlight or focus, and the plan as text.
fn focus<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let entry = r::entry(state, flow.period);
	let intention = entry.map_or("", |entry| entry.intention.as_str());
	let placeholder = match flow.period.scale {
		Scale::Day => "e.g. Send the proposal",
		Scale::Week => "e.g. Get the launch over the line",
		Scale::Month => "e.g. Build the habit of running",
	};
	let mut page = column![
		controls::text_input(placeholder, intention)
			.on_input(|text| msg(Action::Intention(text)))
			.size(metrics.label() + 1.0)
			.width(Fill)
	]
	.spacing(metrics.spacing + 2.0)
	.width(Fill);

	let suggestions: Vec<String> = match flow.period.scale {
		Scale::Day => {
			let mut tasks = r::day_tasks(state, flow.period.first);
			tasks.sort_by_key(|task| tb::metadata(task).priority != Some(Priority::P1));
			tasks
				.into_iter()
				.take(6)
				.map(|task| task.title.clone())
				.collect()
		}
		_ => entry
			.map(|entry| entry.objectives.iter().map(|o| o.text.clone()).collect())
			.unwrap_or_default(),
	};
	if !suggestions.is_empty() {
		page = page.push(muted(
			match flow.period.scale {
				Scale::Day => "Or pick one of today's tasks:",
				_ => "Or pick one of the objectives:",
			},
			metrics,
		));
		page = page.push(chips(
			suggestions
				.into_iter()
				.map(|title| {
					let selected = title == intention;
					chip(
						title.clone(),
						Some(msg(Action::Intention(title))),
						selected,
						metrics,
					)
				})
				.collect(),
		));
	}
	page = page.push(summary_card(state, flow, metrics));
	page.into()
}

fn summary_card<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let summary = r::summary(state, flow);
	card(
		column![
			row![
				text("Summary")
					.size(metrics.label())
					.font(bold())
					.width(Fill),
				chip(
					if state.routines.copied {
						"Copied"
					} else {
						"Copy"
					},
					Some(msg(Action::Copy(summary.clone()))),
					false,
					metrics
				),
			]
			.align_y(Alignment::Center),
			text(summary)
				.size(metrics.label() - 1.0)
				.wrapping(text::Wrapping::WordOrGlyph),
		]
		.spacing(6),
	)
}

// ---------------------------------------------------------------------------
// Daily refresh.

fn refresh_start<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let lists = r::refresh_lists(state);
	let total: usize = lists.iter().map(|(_, tasks)| tasks.len()).sum();
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let mut sections = column![].spacing(6);
	for (section, tasks) in &lists {
		sections = sections.push(
			row![
				column![
					text(section.label()).size(metrics.label()).font(bold()),
					muted(section.about(), metrics),
				]
				.spacing(1)
				.width(Fill),
				container(text(tasks.len().to_string()).size(metrics.label() - 1.0))
					.padding([0, 8])
					.style(controls::tag),
			]
			.align_y(Alignment::Center),
		);
	}
	page = page.push(card(sections));
	if total == 0 {
		page =
			page.push(text("Nothing needs attention. You're all caught up.").size(metrics.label()));
		return page.into();
	}
	page = page.push(muted(
		format!(
			"{total} tasks, one at a time. Each choice is saved as you go; skip any you'd rather leave."
		),
		metrics,
	));
	if lists.iter().any(|(section, tasks)| {
		*section == Section::Unfinished && tasks.iter().any(|task| tb::overdue(task))
	}) {
		page = page.push(
			column![
				text("When a task past its deadline gets a new day, the deadline:")
					.size(metrics.label() - 1.0),
				chips(
					DeadlinePolicy::ALL
						.into_iter()
						.map(|policy| {
							chip(
								policy.label(),
								Some(msg(Action::Deadlines(policy))),
								flow.refresh.deadlines == policy,
								metrics,
							)
						})
						.collect()
				),
			]
			.spacing(4),
		);
	}
	page.into()
}

fn triage<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let refresh = &flow.refresh;
	let Some(card_info) = refresh.card() else {
		return muted("No tasks left.", metrics).into();
	};
	let in_section: Vec<usize> = refresh
		.cards
		.iter()
		.enumerate()
		.filter(|(_, card)| card.section == card_info.section)
		.map(|(index, _)| index)
		.collect();
	let position = in_section
		.iter()
		.position(|&i| i == refresh.index)
		.unwrap_or(0);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	page = page.push(
		column![
			row![
				text(format!(
					"{} · {} of {}",
					card_info.section.label(),
					position + 1,
					in_section.len()
				))
				.size(metrics.label())
				.font(bold())
				.width(Fill),
				muted(
					format!("{} of {} overall", refresh.index + 1, refresh.cards.len()),
					metrics
				),
			]
			.align_y(Alignment::Center),
			meter(refresh.index as u32, refresh.cards.len() as u32),
		]
		.spacing(4),
	);

	let today = r::today(state);
	let Some(task) = r::task(state, &card_info.key) else {
		page = page.push(card(
			column![
				text(&card_info.title)
					.size(metrics.label() + 1.0)
					.font(bold()),
				muted(
					"This task is gone: deleted, or changed somewhere else.",
					metrics
				),
			]
			.spacing(4),
		));
		page = page.push(chip(
			"Next",
			Some(msg(Action::Decide(Decision::Skip))),
			true,
			metrics,
		));
		return page.into();
	};

	// The task itself, large.
	let metadata = tb::metadata(task);
	let mut details = column![
		button(
			text(&task.title)
				.size(metrics.label() + 3.0)
				.font(controls::pending_font(bold(), pending(state, task)))
				.wrapping(text::Wrapping::WordOrGlyph)
				.width(Fill)
		)
		.padding(0)
		.width(Fill)
		.style(controls::ghost)
		.on_press(Message::Event(EventMessage::EventOpened(task.id.clone()))),
		muted(task_meta(state, task), metrics),
	]
	.spacing(4);
	if let Some(created) = metadata.created_at {
		let days = (crate::clock::utc_now() - created).num_days();
		details = details.push(muted(
			match days {
				0 => "Added today".to_owned(),
				1 => "Added yesterday".to_owned(),
				n => format!("Added {n} days ago"),
			},
			metrics,
		));
	}
	let description = task.details.description.trim();
	if !description.is_empty() {
		let short: String = description.chars().take(240).collect();
		details = details.push(
			text(if short.len() < description.len() {
				format!("{short}…")
			} else {
				short
			})
			.size(metrics.label() - 1.0)
			.wrapping(text::Wrapping::WordOrGlyph),
		);
	}
	if let Some(decision) = refresh.decisions.get(&card_info.key) {
		details = details.push(muted(
			format!("Chosen before: {}", decision_label(*decision)),
			metrics,
		));
	}
	details = details.push(chip(
		"Edit details",
		Some(Message::Event(EventMessage::EventOpened(task.id.clone()))),
		false,
		metrics,
	));
	page = page.push(card(details));

	if state.routines.confirm_delete.as_deref() == Some(card_info.key.as_str()) {
		page = page.push(
			container(
				column![
					text("Delete this task for good?")
						.size(metrics.label())
						.font(bold()),
					row![
						button(text("Delete").size(metrics.label()))
							.style(controls::danger)
							.on_press(msg(Action::Decide(Decision::Delete))),
						button(text("Keep it").size(metrics.label()))
							.style(controls::secondary)
							.on_press(msg(Action::CancelDelete)),
					]
					.spacing(6),
				]
				.spacing(6),
			)
			.padding(8)
			.width(Fill)
			.style(controls::card_surface),
		);
		return page.into();
	}

	let decide = |decision| Some(msg(Action::Decide(decision)));
	let tomorrow = today + Duration::days(1);
	let week = Period::of(Scale::Week, today, state.config.calendar.first_day_of_week);
	let mut days = vec![
		chip("Today", decide(Decision::Day(today)), false, metrics),
		chip("Tomorrow", decide(Decision::Day(tomorrow)), false, metrics),
	];
	let mut day = tomorrow + Duration::days(1);
	while day <= week.last() {
		days.push(chip(
			day.format("%a").to_string(),
			decide(Decision::Day(day)),
			false,
			metrics,
		));
		day += Duration::days(1);
	}
	days.push(chip(
		"Next week",
		decide(Decision::Day(week.next().first)),
		false,
		metrics,
	));
	let parked = BacklogSection::ALL
		.into_iter()
		.map(|section| {
			chip(
				section.label(),
				decide(Decision::Backlog(section)),
				false,
				metrics,
			)
		})
		.collect();
	let mut stays = Vec::new();
	match card_info.section {
		Section::Inbox => {
			stays.push(chip("Triage ✓", decide(Decision::Triage), false, metrics));
			stays.push(chip(
				"Leave in inbox",
				decide(Decision::Keep),
				false,
				metrics,
			));
		}
		Section::Today => stays.push(chip(
			"Keep for today",
			decide(Decision::Keep),
			true,
			metrics,
		)),
		Section::Unfinished => {}
	}
	let close = vec![
		chip("Done ✓", decide(Decision::Done), false, metrics),
		chip("Ignore", decide(Decision::Ignore), false, metrics),
		chip("Delete…", decide(Decision::Delete), false, metrics),
	];
	let mut choices = column![
		muted("Do it on", metrics),
		chips(days),
		muted("Park it in the backlog", metrics),
		chips(parked),
	]
	.spacing(4);
	if !stays.is_empty() {
		choices = choices.push(muted("Keep it", metrics)).push(chips(stays));
	}
	choices = choices.push(muted("Close it", metrics)).push(chips(close));
	page = page.push(choices);
	page = page.push(
		row![
			chip("Skip ›", decide(Decision::Skip), false, metrics),
			muted(
				"Keys: T today · M tomorrow · B someday · D done · S skip · ← back",
				metrics
			)
			.width(Fill),
		]
		.spacing(8)
		.align_y(Alignment::Center),
	);

	let next: Vec<&r::Card> = refresh
		.cards
		.iter()
		.skip(refresh.index + 1)
		.take(3)
		.collect();
	if !next.is_empty() {
		let mut list = column![muted("Up next", metrics)].spacing(2);
		for (offset, card) in next.into_iter().enumerate() {
			let title =
				r::task(state, &card.key).map_or(card.title.as_str(), |task| task.title.as_str());
			list = list.push(
				button(
					text(format!("{} · {}", card.section.label(), title))
						.size(metrics.label() - 1.0),
				)
				.padding(0)
				.style(controls::ghost)
				.on_press(msg(Action::Card(refresh.index + 1 + offset))),
			);
		}
		page = page.push(list);
	}
	page.into()
}

fn decision_label(decision: Decision) -> String {
	match decision {
		Decision::Day(day) => format!("planned for {}", short_day(day)),
		Decision::Backlog(section) => format!("backlog, {}", section.label().to_lowercase()),
		Decision::Triage => "triaged".into(),
		Decision::Keep => "kept".into(),
		Decision::Done => "done".into(),
		Decision::Ignore => "ignored".into(),
		Decision::Delete => "deleted".into(),
		Decision::Skip => "skipped".into(),
	}
}

fn refresh_done<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let tally = flow.refresh.tally();
	let mut summary =
		column![heading("This pass", Some(flow.refresh.decided()), metrics)].spacing(4);
	if tally.is_empty() {
		summary = summary.push(muted("No decisions this time.", metrics));
	}
	for (label, count) in tally {
		summary = summary.push(text(format!("{label}: {count}")).size(metrics.label()));
	}
	page = page.push(card(summary));
	let left: Vec<String> = r::refresh_lists(state)
		.into_iter()
		.map(|(section, tasks)| format!("{} {}", tasks.len(), section.label().to_lowercase()))
		.collect();
	page = page.push(muted(format!("Now: {}", left.join(" · ")), metrics));
	if flow.then.is_none()
		&& !Routine::DailyPlan.done(r::entry(state, Period::day(r::today(state))))
	{
		page = page.push(chip(
			"Plan today",
			Some(msg(Action::Start(Routine::DailyPlan))),
			true,
			metrics,
		));
	}
	page.into()
}

// ---------------------------------------------------------------------------
// Reflections.

fn report<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let Some(report) = state
		.routines
		.figures
		.report
		.as_ref()
		.filter(|report| report.period == flow.period)
	else {
		return muted("Working it out…", metrics).into();
	};
	let scale = flow.period.scale;
	let entry = r::entry(state, flow.period);
	let mut page = column![].spacing(metrics.spacing + 4.0).width(Fill);

	let planned = match report.completion() {
		Some(percent) => format!("{percent}% of {} planned", report.planned),
		None => "None planned".into(),
	};
	let habits = match report.habit_rate() {
		Some(rate) => (
			format!("{rate}%"),
			format!(
				"{} kept · {} missed",
				report.habits.kept, report.habits.missed
			),
		),
		None => ("–".into(), "Nothing to check".into()),
	};
	page = page.push(stats(
		vec![
			stat("Done", report.done.len().to_string(), planned, metrics),
			stat(
				"Focus",
				r::duration((report.tracked / 60) as u32),
				format!("{} worked on", count(report.by_task.len(), "task", "tasks")),
				metrics,
			),
			stat(
				"Meetings",
				r::duration(report.meetings.minutes),
				count(report.meetings.count, "event", "events"),
				metrics,
			),
			stat("Habits", habits.0, habits.1, metrics),
		],
		metrics,
	));

	// The day's own plan and highlight.
	if scale == Scale::Day {
		if let Some(plan) = report.plan {
			page = page.push(
				text(format!(
					"This morning you planned {} ({}); {} of them done.",
					count(plan.planned, "task", "tasks"),
					r::duration(plan.minutes),
					plan.done
				))
				.size(metrics.label()),
			);
		}
		if let Some(entry) = entry.filter(|entry| !entry.intention.trim().is_empty()) {
			page = page.push(card(
				column![
					muted("Today's highlight", metrics),
					text(entry.intention.trim())
						.size(metrics.label() + 1.0)
						.font(bold()),
					row![
						muted("Did it happen?", metrics),
						chip(
							"Yes",
							Some(msg(Action::IntentionMet(Some(true)))),
							entry.intention_met == Some(true),
							metrics
						),
						chip(
							"Not this time",
							Some(msg(Action::IntentionMet(Some(false)))),
							entry.intention_met == Some(false),
							metrics
						),
					]
					.spacing(6)
					.align_y(Alignment::Center),
				]
				.spacing(4),
			));
		}
	} else {
		// Done per day for a week; per week for a month.
		let values: Vec<(String, u32, String)> = match scale {
			Scale::Week => report
				.days
				.iter()
				.map(|day| {
					(
						day.day.format("%a").to_string(),
						day.done as u32,
						format!(
							"{}: {} done · {} focus · {} meetings",
							short_day(day.day),
							day.done,
							r::duration(day.tracked_minutes),
							r::duration(day.meeting_minutes)
						),
					)
				})
				.collect(),
			_ => {
				let first_day = state.config.calendar.first_day_of_week;
				let mut weeks: Vec<(Period, u32, u32)> = Vec::new();
				for day in &report.days {
					let week = Period::of(Scale::Week, day.day, first_day);
					match weeks.last_mut() {
						Some((last, done, focus)) if *last == week => {
							*done += day.done as u32;
							*focus += day.tracked_minutes;
						}
						_ => weeks.push((week, day.done as u32, day.tracked_minutes)),
					}
				}
				weeks
					.into_iter()
					.map(|(week, done, focus)| {
						(
							week.first.format("%b %-d").to_string(),
							done,
							format!(
								"Week of {}: {done} done · {} focus",
								week.label(),
								r::duration(focus)
							),
						)
					})
					.collect()
			}
		};
		page = page.push(
			column![
				heading(
					if scale == Scale::Week {
						"Tasks done each day"
					} else {
						"Tasks done each week"
					},
					None,
					metrics
				),
				columns(values, metrics),
			]
			.spacing(6),
		);
		let mut habits_line = Vec::new();
		if let Some(average) = report.average_rating() {
			habits_line.push(format!("Days rated {average:.1} of 5 on average"));
		}
		let met = report
			.intentions
			.iter()
			.filter(|(_, _, met)| *met == Some(true))
			.count();
		if !report.intentions.is_empty() {
			habits_line.push(format!(
				"{met} of {} daily highlights met",
				report.intentions.len()
			));
		}
		habits_line.push(format!(
			"Planned {} of {} days · refreshed {} · reflected {}",
			report.planned_days, report.elapsed_days, report.refreshed_days, report.reflected_days
		));
		page = page.push(card(
			column(
				habits_line
					.into_iter()
					.map(|line| text(line).size(metrics.label() - 1.0).into()),
			)
			.spacing(3),
		));
		if !report.highlights.is_empty() {
			let mut list = column![heading(
				"Highlights of the days",
				Some(report.highlights.len()),
				metrics
			)]
			.spacing(4);
			for (day, highlight) in &report.highlights {
				list = list.push(
					text(format!("★ {} · {}", highlight.title, day.format("%a %-d")))
						.size(metrics.label() - 1.0)
						.wrapping(text::Wrapping::WordOrGlyph),
				);
			}
			page = page.push(list);
		}
	}

	// What got done, to pick highlights from.
	let mut done = column![heading("Finished", Some(report.done.len()), metrics)].spacing(4);
	if report.done.is_empty() {
		done = done.push(muted("Nothing ticked off.", metrics));
	} else {
		done = done.push(muted("Star the ones worth remembering.", metrics));
	}
	let open = state.routines.expanded.contains(&Group::Done);
	for task in report.done.iter().take(if open { usize::MAX } else { 8 }) {
		let starred = entry.is_some_and(|entry| entry.is_highlight(&task.key));
		let mut figures = Vec::new();
		if scale != Scale::Day {
			figures.push(task.day.format("%a %-d").to_string());
		}
		if task.tracked > 0 {
			figures.push(format!(
				"{} worked",
				r::duration((task.tracked / 60) as u32)
			));
		} else if task.estimate > 0 {
			figures.push(format!("{} estimated", r::duration(task.estimate)));
		}
		done = done.push(
			row![
				button(text(if starred { "★" } else { "☆" }).size(metrics.label() + 2.0))
					.padding([0, 4])
					.style(controls::ghost)
					.on_press(msg(Action::Highlight(task.key.clone(), task.title.clone()))),
				button(
					text(&task.title)
						.size(metrics.label())
						.wrapping(text::Wrapping::WordOrGlyph)
						.width(Fill)
				)
				.padding(0)
				.width(Fill)
				.style(controls::ghost)
				.on_press(Message::Event(EventMessage::EventOpened(task.id.clone()))),
				muted(figures.join(" · "), metrics),
			]
			.spacing(4)
			.align_y(Alignment::Center),
		);
	}
	if report.done.len() > 8 {
		done = done.push(
			button(
				text(if open {
					"Show fewer".to_owned()
				} else {
					format!("Show all {}", report.done.len())
				})
				.size(metrics.label() - 1.0),
			)
			.padding([2, 0])
			.style(controls::ghost)
			.on_press(msg(Action::Expand(Group::Done))),
		);
	}
	page = page.push(done);

	// Where the time went.
	let projects: Vec<_> = report
		.by_project
		.iter()
		.filter(|p| p.seconds > 0)
		.take(5)
		.map(|p| {
			let (color, name) = project_name(state, p.project);
			(color, name, p.seconds, r::duration((p.seconds / 60) as u32))
		})
		.collect();
	if !projects.is_empty() {
		page = page.push(
			column![
				heading("Focus by project", None, metrics),
				ranked(projects, metrics)
			]
			.spacing(6),
		);
	}
	let tasks: Vec<_> = report
		.by_task
		.iter()
		.take(5)
		.map(|t| {
			(
				None,
				t.title.clone(),
				t.seconds,
				r::duration((t.seconds / 60) as u32),
			)
		})
		.collect();
	if !tasks.is_empty() {
		page = page.push(
			column![
				heading("Focus by task", None, metrics),
				ranked(tasks, metrics)
			]
			.spacing(6),
		);
	}

	let mut other = Vec::new();
	if report.blocks.count > 0 {
		other.push(format!(
			"{}, {}",
			count(report.blocks.count, "time block", "time blocks"),
			r::duration(report.blocks.minutes)
		));
	}
	if report.distractions.count > 0 {
		other.push(format!(
			"{}, {} lost",
			count(report.distractions.count, "distraction", "distractions"),
			r::duration(report.distractions.minutes)
		));
	}
	if let Some(sleep) = report.sleep {
		other.push(format!(
			"Slept {} a night on average ({} nights)",
			r::duration(sleep.average_minutes),
			sleep.nights
		));
	}
	if report.open > 0 {
		other.push(format!(
			"{} still open",
			count(report.open, "planned task", "planned tasks")
		));
	}
	if report.ignored > 0 {
		other.push(format!("{} let go", report.ignored));
	}
	if !report.habits.best.is_empty() {
		let best: Vec<String> = report
			.habits
			.best
			.iter()
			.map(|(name, days)| format!("{name} ({days})"))
			.collect();
		other.push(format!("Best habits: {}", best.join(", ")));
	}
	if !other.is_empty() {
		page = page.push(card(
			column(
				other
					.into_iter()
					.map(|line| text(line).size(metrics.label() - 1.0).into()),
			)
			.spacing(3),
		));
	}
	if report.known_days < report.elapsed_days {
		page = page.push(muted(
			format!(
				"Meetings and blocks cover the {} of {} days whose calendar has been loaded.",
				report.known_days, report.elapsed_days
			),
			metrics,
		));
	}
	page.into()
}

fn unfinished<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let day = flow.period.first;
	let tomorrow = day + Duration::days(1);
	let tasks = r::day_tasks(state, day);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	if tasks.is_empty() {
		page = page.push(text("Nothing left over. Nicely done.").size(metrics.label()));
		return page.into();
	}
	let keys: Vec<String> = tasks.iter().map(|task| tb::task_key(task)).collect();
	page = page.push(chip(
		format!(
			"Move all {} to tomorrow",
			count(tasks.len(), "task", "tasks")
		),
		Some(msg(Action::Tasks(keys, Change::Day(tomorrow)))),
		true,
		metrics,
	));
	for task in tasks {
		page = page.push(task_row(
			state,
			task,
			vec![
				chip(
					"Tomorrow",
					change(task, Change::Day(tomorrow)),
					false,
					metrics,
				),
				chip(
					"Backlog",
					change(task, Change::Backlog(BacklogSection::NextWeekOrTwo)),
					false,
					metrics,
				),
				chip("Done ✓", change(task, Change::Complete), false, metrics),
			],
			metrics,
		));
	}
	page.into()
}

fn objectives_review<'a>(
	state: &'a State,
	flow: &'a Flow,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let entry = r::entry(state, flow.period);
	let next = r::entry(state, flow.period.next());
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let objectives = entry
		.map(|entry| entry.objectives.as_slice())
		.unwrap_or_default();
	if objectives.is_empty() {
		page = page.push(muted(
			format!(
				"No objectives were set for this {}.",
				flow.period.scale.noun()
			),
			metrics,
		));
		return page.into();
	}
	let noun = flow.period.scale.noun();
	for objective in objectives {
		let carried = next.is_some_and(|next| {
			next.objectives
				.iter()
				.any(|o| o.text.trim() == objective.text.trim())
		});
		let mut actions = vec![
			chip(
				"Achieved",
				Some(msg(Action::ObjectiveDone(objective.id, true))),
				objective.done,
				metrics,
			),
			chip(
				"Not yet",
				Some(msg(Action::ObjectiveDone(objective.id, false))),
				!objective.done,
				metrics,
			),
		];
		if !objective.done {
			actions.push(chip(
				if carried {
					format!("In next {noun}'s plan")
				} else {
					format!("Carry to next {noun}")
				},
				(!carried).then(|| msg(Action::CarryForward(objective.id))),
				false,
				metrics,
			));
		}
		page = page.push(card(
			column![
				text(&objective.text)
					.size(metrics.label())
					.font(bold())
					.wrapping(text::Wrapping::WordOrGlyph),
				chips(actions),
			]
			.spacing(4),
		));
	}
	page.into()
}

fn reflect<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let rating = r::entry(state, flow.period).and_then(|entry| entry.rating);
	let mut page = column![].spacing(metrics.spacing + 4.0).width(Fill);
	let labels = ["Rough", "Meh", "Okay", "Good", "Great"];
	page = page.push(
		column![
			text("Overall").size(metrics.label()).font(bold()),
			chips(
				labels
					.iter()
					.enumerate()
					.map(|(index, label)| {
						let value = index as u8 + 1;
						chip(
							format!("{value} {label}"),
							Some(msg(Action::Rating(value))),
							rating == Some(value),
							metrics,
						)
					})
					.collect()
			),
		]
		.spacing(4),
	);
	for (id, question) in flow.routine.prompts() {
		let Some(content) = state.routines.editor(id) else {
			continue;
		};
		let prompt = *id;
		page = page.push(
			column![
				text(*question).size(metrics.label()).font(bold()),
				controls::text_editor(content)
					.on_action(move |action| msg(Action::Prompt(prompt, action)))
					.size(metrics.label())
					.height(84),
			]
			.spacing(4),
		);
	}
	page.into()
}

fn wrapup<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	match flow.period.scale {
		Scale::Day => {
			let tomorrow = flow.period.first + Duration::days(1);
			let tasks = r::day_tasks(state, tomorrow);
			let mut next = column![heading("Tomorrow", Some(tasks.len()), metrics)].spacing(4);
			if tasks.is_empty() {
				next = next.push(muted(
					"Nothing planned yet. The daily plan will help in the morning.",
					metrics,
				));
			}
			for task in tasks.iter().take(5) {
				next = next.push(text(format!("• {}", task.title)).size(metrics.label() - 1.0));
			}
			page = page.push(card(next));
		}
		scale => {
			let next = flow.period.next();
			let carried = r::entry(state, next).map_or(0, |entry| entry.objectives.len());
			page = page.push(muted(
				format!(
					"{} objectives are waiting in next {}'s plan.",
					carried,
					scale.noun()
				),
				metrics,
			));
		}
	}
	page = page.push(summary_card(state, flow, metrics));
	page.into()
}

// ---------------------------------------------------------------------------
// Weekly and monthly plans.

fn look_back<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let previous = flow.period.previous();
	let noun = flow.period.scale.noun();
	let entry = r::entry(state, previous);
	let current = r::entry(state, flow.period);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let hour = state.config.calendar.day_start_hour();
	let done = state
		.calendar
		.events
		.iter()
		.filter(|task| {
			r::countable(task)
				&& tb::task_status(task) == tb::TaskStatus::Completed
				&& crate::routines::report::done_day(task, hour)
					.is_some_and(|day| previous.contains(day))
		})
		.count();
	let mut summary = column![heading(
		format!("Last {noun} · {}", previous.label()),
		None,
		metrics
	)]
	.spacing(4);
	summary = summary.push(text(format!("{done} tasks finished.")).size(metrics.label()));
	if let Some(entry) = entry {
		if let Some(rating) = entry.rating {
			summary = summary.push(muted(format!("You rated it {rating} of 5."), metrics));
		}
		if !entry.intention.trim().is_empty() {
			summary = summary.push(muted(format!("Focus: {}", entry.intention.trim()), metrics));
		}
		let change = entry
			.answer(Routine::carried_prompt(flow.period.scale))
			.trim();
		if !change.is_empty() {
			summary =
				summary.push(text(format!("You wanted to change: {change}")).size(metrics.label()));
		}
	} else {
		summary = summary.push(muted(
			format!("No plan or reflection for last {noun}."),
			metrics,
		));
	}
	page = page.push(card(summary));

	let objectives = entry
		.map(|entry| entry.objectives.as_slice())
		.unwrap_or_default();
	if !objectives.is_empty() {
		let mut list = column![heading(
			format!("Last {noun}'s objectives"),
			Some(objectives.len()),
			metrics
		)]
		.spacing(6);
		for objective in objectives {
			let carried = current.is_some_and(|c| {
				c.objectives
					.iter()
					.any(|o| o.text.trim() == objective.text.trim())
			});
			let mut line = row![
				text(format!(
					"{} {}",
					if objective.done { "✓" } else { "○" },
					objective.text
				))
				.size(metrics.label())
				.wrapping(text::Wrapping::WordOrGlyph)
				.width(Fill)
			]
			.spacing(6)
			.align_y(Alignment::Center);
			if !objective.done {
				line = line.push(chip(
					if carried {
						"Carried over"
					} else {
						"Carry over"
					},
					(!carried).then(|| msg(Action::CarryOver(objective.id))),
					false,
					metrics,
				));
			}
			list = list.push(line);
		}
		page = page.push(list);
	}
	page.into()
}

fn objectives<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let entry = r::entry(state, flow.period);
	let objectives = entry
		.map(|entry| entry.objectives.as_slice())
		.unwrap_or_default();
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	for objective in objectives {
		let id = objective.id;
		page = page.push(
			row![
				controls::text_input("Objective", &objective.text)
					.on_input(move |text| msg(Action::ObjectiveText(id, text)))
					.size(metrics.label())
					.width(Fill),
				button(text("×").size(metrics.label() + 2.0))
					.padding([0, 8])
					.style(controls::ghost)
					.on_press(msg(Action::RemoveObjective(id))),
			]
			.spacing(4)
			.align_y(Alignment::Center),
		);
	}
	page = page.push(
		row![
			controls::text_input("Add an objective", &state.routines.objective_draft)
				.on_input(|text| msg(Action::ObjectiveDraft(text)))
				.on_submit(msg(Action::AddObjective))
				.size(metrics.label())
				.width(Fill),
			button(text("Add").size(metrics.label()))
				.style(controls::primary)
				.on_press(msg(Action::AddObjective)),
		]
		.spacing(6)
		.align_y(Alignment::Center),
	);
	page = page.push(muted(
		if objectives.len() > 5 {
			"That's a lot. Three to five keeps them meaningful."
		} else {
			"Outcomes, not tasks: three to five is plenty."
		},
		metrics,
	));

	// What the objectives should serve.
	match flow.period.scale {
		Scale::Week => {
			let month = Period::of(
				Scale::Month,
				flow.period.first,
				state.config.calendar.first_day_of_week,
			);
			if let Some(month) = r::entry(state, month).filter(|entry| !entry.objectives.is_empty())
			{
				let mut list =
					column![heading("This month's objectives", None, metrics)].spacing(4);
				for objective in &month.objectives {
					list = list
						.push(text(format!("• {}", objective.text)).size(metrics.label() - 1.0));
				}
				page = page.push(card(list));
			}
		}
		_ => {
			let goals: Vec<(String, Vec<String>)> = state
				.config
				.projects
				.areas
				.iter()
				.filter(|area| !area.goals.is_empty())
				.map(|area| {
					(
						area.name.clone(),
						area.goals.iter().map(|goal| goal.name.clone()).collect(),
					)
				})
				.collect();
			if !goals.is_empty() {
				let mut list = column![heading("Your goals", None, metrics)].spacing(4);
				for (area, goals) in goals {
					list = list.push(text(area).size(metrics.label() - 1.0).font(bold()));
					for goal in goals {
						list = list.push(text(format!("• {goal}")).size(metrics.label() - 1.0));
					}
				}
				page = page.push(card(list));
			}
		}
	}
	page.into()
}

fn schedule<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	match flow.period.scale {
		Scale::Month => month_backlog(state, flow, metrics),
		_ => week_days(state, flow, metrics),
	}
}

fn week_days<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let today = r::today(state);
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let mut days = column![heading("The week", None, metrics)].spacing(6);
	for day in flow.period.days() {
		let tasks = r::day_tasks(state, day);
		let (planned, _) = r::estimated(&tasks);
		let free = r::free_minutes(state, day);
		let past = day < today;
		let meetings: i64 = state
			.routines
			.figures
			.busy
			.get(&day)
			.into_iter()
			.flatten()
			.map(|(start, end)| (*end - *start).num_minutes())
			.sum();
		let mut line = column![
			row![
				text(day.format("%a %-d").to_string())
					.size(metrics.label())
					.font(bold())
					.width(Length::Fixed(56.0)),
				muted(
					if past {
						format!("{} left open", tasks.len())
					} else {
						let mut parts = vec![count(tasks.len(), "task", "tasks")];
						if planned > 0 {
							parts.push(format!("{} planned", r::duration(planned)));
						}
						parts.push(format!("{} free", r::duration(free)));
						if meetings > 0 {
							parts.push(format!("{} in meetings", r::duration(meetings as u32)));
						}
						parts.join(" · ")
					},
					metrics
				)
				.width(Fill),
			]
			.align_y(Alignment::Center),
		]
		.spacing(3);
		if !past {
			line = line.push(meter(planned, free));
		}
		days = days.push(line);
	}
	page = page.push(card(days));

	let first = today.max(flow.period.first);
	let choices = move |task: &Event| -> Vec<Element<'a, Message>> {
		let mut day = first;
		let mut out = Vec::new();
		while day <= flow.period.last() {
			let label = if day == today {
				"Today".to_owned()
			} else {
				day.format("%a").to_string()
			};
			out.push(chip(label, change(task, Change::Day(day)), false, metrics));
			day += Duration::days(1);
		}
		out
	};
	let hour = state.config.calendar.day_start_hour();
	let undated = |tasks: Vec<&'a Event>| -> Vec<&'a Event> {
		tasks
			.into_iter()
			.filter(|task| r::task_day(task, hour).is_none())
			.collect()
	};
	let groups = [
		group(
			state,
			Group::NextWeeks,
			"Backlog · next week or two",
			r::backlog(state, BacklogSection::NextWeekOrTwo),
			choices,
			metrics,
		),
		group(
			state,
			Group::DueSoon,
			"Due this week, no day yet",
			undated(r::due_by(state, flow.period.last())),
			choices,
			metrics,
		),
		group(
			state,
			Group::Triaged,
			"Triaged",
			undated(crate::inbox::triaged_tasks(state)),
			choices,
			metrics,
		),
		group(
			state,
			Group::Unfinished,
			"Unfinished from earlier days",
			r::unfinished(state, today),
			choices,
			metrics,
		),
	];
	for group in groups.into_iter().flatten() {
		page = page.push(group);
	}
	page = page.push(chip(
		"Open the week planner",
		Some(msg(Action::OpenPlanner)),
		false,
		metrics,
	));
	page.into()
}

fn month_backlog<'a>(state: &'a State, flow: &'a Flow, metrics: &Metrics) -> Element<'a, Message> {
	let mut page = column![].spacing(metrics.spacing + 2.0).width(Fill);
	let first_day = state.config.calendar.first_day_of_week;
	let mut weeks = column![heading("The month", None, metrics)].spacing(4);
	let mut week = Period::of(Scale::Week, flow.period.first, first_day);
	while week.first <= flow.period.last() {
		let tasks: Vec<&Event> = week
			.days()
			.into_iter()
			.filter(|day| flow.period.contains(*day))
			.flat_map(|day| r::day_tasks(state, day))
			.collect();
		let (minutes, _) = r::estimated(&tasks);
		weeks = weeks.push(
			row![
				text(week.label()).size(metrics.label() - 1.0).width(Fill),
				muted(
					format!(
						"{} · {}",
						count(tasks.len(), "task", "tasks"),
						r::duration(minutes)
					),
					metrics
				),
			]
			.spacing(6),
		);
		week = week.next();
	}
	page = page.push(card(weeks));

	let horizon = |options: &'static [BacklogSection]| {
		move |task: &Event| -> Vec<Element<'a, Message>> {
			options
				.iter()
				.map(|section| {
					chip(
						section.label(),
						change(task, Change::Horizon(*section)),
						false,
						metrics,
					)
				})
				.collect()
		}
	};
	let groups = [
		group(
			state,
			Group::NextMonth,
			"Parked for next month",
			r::backlog(state, BacklogSection::NextMonth),
			horizon(&[
				BacklogSection::NextWeekOrTwo,
				BacklogSection::NextQuarter,
				BacklogSection::Someday,
			]),
			metrics,
		),
		group(
			state,
			Group::NextQuarter,
			"Parked for next quarter",
			r::backlog(state, BacklogSection::NextQuarter),
			horizon(&[BacklogSection::NextMonth, BacklogSection::NextWeekOrTwo]),
			metrics,
		),
		group(
			state,
			Group::DueSoon,
			"Due this month",
			r::due_by(state, flow.period.last()),
			|_| Vec::new(),
			metrics,
		),
	];
	let mut any = false;
	for group in groups.into_iter().flatten() {
		any = true;
		page = page.push(group);
	}
	if !any {
		page = page.push(muted(
			"Nothing parked for the next month or quarter.",
			metrics,
		));
	}
	page = page.push(chip(
		"Open the backlog",
		Some(msg(Action::OpenPlanner)),
		false,
		metrics,
	));
	page.into()
}
