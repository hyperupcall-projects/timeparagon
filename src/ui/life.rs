//! The Life tab. The main window shows one of five pages (Focus, Areas,
//! Goals, Timeline, Review) and the side panel shows whatever was picked
//! from them: a life area, goal, objective or project, each linking on to the
//! others.

use chrono::{Datelike, NaiveDate};
use iced::widget::{Space, button, column, container, progress_bar, row, text};
use iced::{Alignment, Border, Color, Element, Fill, Length, Padding, Theme};
use timeparagon_core::notes::Pastel;

use crate::app::{Message, State};
use crate::calendar::colors;
use crate::config::{LifeArea, Measure, Objective, Pace, ProjectsSettings, Status};
use crate::life::{
	Action, GoalFilter, Horizon, Open, Page, Selection, VisionMode, habit_score, project_tasks,
};

use super::calendar::{color_circle, swatch_button};
use super::controls;
use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// What an area or project without a color of its own is drawn with.
const NO_COLOR: Color = Color::from_rgb(0.62, 0.62, 0.64);

/// Room for an area's, goal's or objective's page in the side window.
pub const PANEL_WIDTH: f32 = 560.0;

/// An objective not checked in on for this long needs attention.
const STALE_CHECK_IN_DAYS: i64 = 14;

/// How far ahead Focus looks for what is due.
const DUE_SOON_DAYS: i64 = 30;

fn msg(action: Action) -> Message {
	Message::Life(action)
}

fn open_msg(selection: Selection) -> Message {
	msg(Action::Open(selection))
}

fn area_color(area: &LifeArea) -> Color {
	area.color().unwrap_or(NO_COLOR)
}

fn plural(count: usize, one: &str, many: &str) -> String {
	format!("{count} {}", if count == 1 { one } else { many })
}

fn percent(value: f64) -> String {
	format!("{:.0}%", value * 100.0)
}

// ---------------------------------------------------------------------------
// Small pieces shared by the pages and the panel.

fn label<'a>(caption: impl text::IntoFragment<'a>, metrics: &Metrics) -> text::Text<'a, Theme> {
	text(caption)
		.size(metrics.label() - 1.0)
		.style(text::secondary)
		.wrapping(text::Wrapping::WordOrGlyph)
}

fn heading<'a>(
	caption: impl Into<String>,
	count: Option<usize>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	controls::section_heading(caption, count, metrics.label() + 2.0).into()
}

fn gap<'a>() -> Element<'a, Message> {
	Space::new().height(Length::Fixed(6.0)).into()
}

/// A progress bar in `color`; empty when progress cannot be told.
fn bar<'a>(value: Option<f64>, color: Color, girth: f32) -> Element<'a, Message> {
	progress_bar(0.0..=1.0, value.unwrap_or(0.0).clamp(0.0, 1.0) as f32)
		.girth(girth)
		.style(move |theme: &Theme| progress_bar::Style {
			background: controls::edge_color(theme).into(),
			bar: color.into(),
			border: Border {
				radius: (girth / 2.0).into(),
				..Border::default()
			},
		})
		.into()
}

/// A bar with its percentage beside it.
fn progress_line<'a>(value: Option<f64>, color: Color, metrics: &Metrics) -> Element<'a, Message> {
	row![
		container(bar(value, color, 6.0)).width(Fill),
		text(value.map(percent).unwrap_or_else(|| "–".into()))
			.size(metrics.label() - 1.0)
			.width(Length::Fixed(38.0))
			.align_x(iced::alignment::Horizontal::Right),
	]
	.spacing(8)
	.align_y(Alignment::Center)
	.into()
}

/// A rounded tag in one of the theme's tones.
fn chip<'a>(caption: impl Into<String>, tone: Tone, size: f32) -> Element<'a, Message> {
	container(text(caption.into()).size(size - 2.0))
		.padding([1, 7])
		.style(move |theme: &Theme| {
			let p = theme.extended_palette();
			let pair = match tone {
				Tone::Good => p.success.weak,
				Tone::Warn => p.warning.weak,
				Tone::Bad => p.danger.weak,
				Tone::Accent => p.primary.weak,
				Tone::Quiet => p.background.strong,
			};
			container::Style {
				background: Some(pair.color.into()),
				text_color: Some(pair.text),
				border: Border {
					radius: 8.0.into(),
					..Border::default()
				},
				..container::Style::default()
			}
		})
		.into()
}

#[derive(Debug, Clone, Copy)]
enum Tone {
	Good,
	Warn,
	Bad,
	Accent,
	Quiet,
}

fn pace_tone(pace: Pace) -> Tone {
	match pace {
		Pace::Done | Pace::OnTrack => Tone::Good,
		Pace::Behind => Tone::Warn,
		Pace::Overdue => Tone::Bad,
		Pace::Open => Tone::Accent,
		Pace::Paused | Pace::Dropped => Tone::Quiet,
	}
}

fn pace_chip<'a>(pace: Pace, size: f32) -> Element<'a, Message> {
	chip(pace.label(), pace_tone(pace), size)
}

fn status_tone(status: Status) -> Tone {
	match status {
		Status::Active => Tone::Accent,
		Status::Done => Tone::Good,
		Status::Paused | Status::Dropped => Tone::Quiet,
	}
}

/// When something is due, relative to today.
fn due_label(due: NaiveDate, today: NaiveDate) -> String {
	let days = (due - today).num_days();
	match days {
		d if d < 0 => format!("{} overdue", plural((-d) as usize, "day", "days")),
		0 => "Due today".into(),
		1 => "Due tomorrow".into(),
		2..=6 => format!("Due in {days} days"),
		_ if due.year() == today.year() => format!("Due {}", due.format("%b %-d")),
		_ => format!("Due {}", due.format("%b %-d, %Y")),
	}
}

/// What an objective's measure reads right now.
fn measure_detail(state: &State, objective: &Objective) -> Option<String> {
	let unit = |value: f64| {
		let number = crate::life::format_number(value);
		if objective.unit.trim().is_empty() {
			number
		} else {
			format!("{number} {}", objective.unit.trim())
		}
	};
	match objective.measure {
		Measure::None => None,
		Measure::Number => match (objective.current(), objective.target) {
			(Some(current), Some(target)) => Some(format!("{} of {}", unit(current), unit(target))),
			(None, Some(target)) => Some(format!("Target {}", unit(target))),
			_ => Some("Set a start and a target".into()),
		},
		Measure::Milestones => {
			let done = objective.milestones.iter().filter(|m| m.done).count();
			Some(format!(
				"{done} of {}",
				plural(objective.milestones.len(), "milestone", "milestones")
			))
		}
		Measure::Habits => {
			let score =
				crate::life::mean_habit_score(state, objective.habits.iter().map(String::as_str));
			Some(match score {
				Some(score) => format!(
					"{} · {} strength",
					plural(objective.habits.len(), "habit", "habits"),
					percent(score)
				),
				None => "No habits yet".into(),
			})
		}
	}
}

fn score_of(state: &State) -> impl Fn(&str) -> Option<f64> + '_ {
	move |habit| habit_score(state, habit)
}

/// Where an objective sits: its goal and area.
fn objective_context(area: &LifeArea, objective: &Objective) -> String {
	match area
		.objective_goal(objective)
		.and_then(|goal| area.goal(goal))
	{
		Some(goal) => format!("{} · {}", goal.name, area.name),
		None => area.name.clone(),
	}
}

/// An objective as a card: what it serves, how far along it is, and when
/// it is due. Opens it in the panel.
fn objective_card<'a>(
	state: &'a State,
	area: &'a LifeArea,
	objective: &'a Objective,
	note: Option<String>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let today = crate::life::today();
	let score = score_of(state);
	let progress = objective.progress(&score);
	let pace = objective.pace(today, &score);

	let mut footer = row![].spacing(8).align_y(Alignment::Center);
	if let Some(detail) = measure_detail(state, objective) {
		footer = footer.push(label(detail, metrics).width(Fill));
	} else {
		footer = footer.push(Space::new().width(Fill));
	}
	if let Some(due) = objective.due {
		footer = footer.push(label(due_label(due, today), metrics));
	}

	let mut content = column![
		row![
			color_circle(area_color(area)),
			text(&objective.name)
				.size(size + 1.0)
				.wrapping(text::Wrapping::WordOrGlyph)
				.width(Fill),
			pace_chip(pace, size),
		]
		.spacing(8)
		.align_y(Alignment::Center),
		label(objective_context(area, objective), metrics),
	]
	.spacing(4)
	.width(Fill);
	if objective.measure != Measure::None || objective.status == Status::Done {
		content = content.push(progress_line(progress, area_color(area), metrics));
	}
	content = content.push(footer);
	if let Some(note) = note {
		content = content.push(
			text(note)
				.size(size - 1.0)
				.style(|theme: &Theme| text::Style {
					color: Some(theme.extended_palette().warning.strong.color),
				}),
		);
	}

	button(content)
		.on_press(open_msg(Selection::Objective(objective.id)))
		.padding([10, 12])
		.width(Fill)
		.style(controls::secondary)
		.into()
}

/// A compact row for an objective inside a goal or area.
fn objective_row<'a>(
	state: &'a State,
	area: &'a LifeArea,
	objective: &'a Objective,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let today = crate::life::today();
	let score = score_of(state);
	let progress = objective.progress(&score);
	let pace = objective.pace(today, &score);
	let mut right = row![].spacing(6).align_y(Alignment::Center);
	if objective.focus {
		right = right.push(chip("Focus", Tone::Accent, size));
	}
	right = right.push(pace_chip(pace, size));

	let mut words = column![
		row![
			text(&objective.name)
				.size(size)
				.wrapping(text::Wrapping::WordOrGlyph)
				.width(Fill),
			right,
		]
		.spacing(6)
		.align_y(Alignment::Center)
	]
	.spacing(4)
	.width(Fill);
	if objective.measure != Measure::None || objective.status == Status::Done {
		words = words.push(progress_line(progress, area_color(area), metrics));
	}
	let mut details = Vec::new();
	if let Some(detail) = measure_detail(state, objective) {
		details.push(detail);
	}
	if let Some(due) = objective.due {
		details.push(due_label(due, today));
	}
	if !details.is_empty() {
		words = words.push(label(details.join(" · "), metrics));
	}

	button(words)
		.on_press(open_msg(Selection::Objective(objective.id)))
		.padding([6, 10])
		.width(Fill)
		.style(controls::secondary)
		.into()
}

fn empty_state<'a>(title: &'a str, hint: &'a str, metrics: &Metrics) -> Element<'a, Message> {
	container(
		column![
			text(title).size(metrics.label() + 1.0),
			label(hint, metrics),
		]
		.spacing(4),
	)
	.padding([12, 14])
	.width(Fill)
	.style(controls::row_surface)
	.into()
}

/// A number with a caption under it.
fn stat<'a>(value: String, caption: &'a str, metrics: &Metrics) -> Element<'a, Message> {
	container(
		column![
			text(value).size(metrics.label() + 9.0),
			label(caption, metrics),
		]
		.spacing(2),
	)
	.padding([8, 12])
	.width(Fill)
	.style(controls::row_surface)
	.into()
}

/// A habit's color, which depends on whether the theme is dark.
fn habit_dot<'a>(index: u8) -> Element<'a, Message> {
	dot(index, true, 10.0)
}

fn dot<'a>(index: u8, filled: bool, size: f32) -> Element<'a, Message> {
	container(Space::new().width(size).height(size))
		.style(move |theme: &Theme| container::Style {
			background: Some(if filled {
				timeparagon_core::habits::palette_color(index, theme.extended_palette().is_dark)
					.into()
			} else {
				controls::edge_color(theme).into()
			}),
			border: Border {
				radius: controls::RADIUS_ROW.into(),
				..Border::default()
			},
			..container::Style::default()
		})
		.into()
}

/// The last seven days of a habit, oldest first.
fn week_dots<'a>(state: &'a State, habit: &'a crate::habits::Habit) -> Element<'a, Message> {
	let today = state.habits.today();
	let computed = state.habits.computed(&habit.id);
	let mut dots = row![].spacing(3).align_y(Alignment::Center);
	for back in (0..7).rev() {
		let day = today - chrono::Days::new(back);
		let done = computed.is_some_and(|c| c.status(day).completed());
		dots = dots.push(dot(habit.color, done, 9.0));
	}
	dots.into()
}

/// A row of choices where the chosen one is highlighted.
fn segmented<'a, T: Copy + PartialEq + 'a>(
	options: &[T],
	current: T,
	caption: impl Fn(T) -> &'static str,
	on_press: impl Fn(T) -> Message,
	size: f32,
) -> Element<'a, Message> {
	let mut line = row![].spacing(4);
	for &option in options {
		line = line.push(
			button(text(caption(option)).size(size - 1.0))
				.on_press(on_press(option))
				.style(controls::nav(option == current)),
		);
	}
	line.wrap().into()
}

fn color_picker<'a>(
	current: Option<Color>,
	on_pick: impl Fn(Option<Color>) -> Message,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let mut swatches = row![].spacing(6);
	for color in colors::palette() {
		swatches = swatches.push(swatch_button(
			color,
			current == Some(color),
			on_pick(Some(color)),
		));
	}
	let no_color = button(text("No color").size(size - 1.0).center())
		.on_press(on_pick(None))
		.style(if current.is_none() {
			controls::primary
		} else {
			controls::secondary
		});
	column![swatches.wrap().vertical_spacing(6.0), no_color]
		.spacing(6)
		.into()
}

/// Remove, and then the yes or no that removing waits for.
fn remove_control<'a>(
	open: &'a Open,
	what: &'static str,
	consequence: &'static str,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	if open.confirm_remove {
		column![
			text(format!("Remove this {what}? {consequence}")).size(size),
			row![
				button(text("Remove").size(size))
					.on_press(msg(Action::ConfirmRemove))
					.style(controls::danger),
				button(text("Keep").size(size))
					.on_press(msg(Action::CancelRemove))
					.style(controls::secondary),
			]
			.spacing(6),
		]
		.spacing(4)
		.into()
	} else {
		button(text(format!("Remove {what}")).size(size))
			.on_press(msg(Action::Remove))
			.style(controls::danger)
			.into()
	}
}

/// A text box and an add button beside it.
fn add_row<'a>(
	placeholder: &str,
	value: &str,
	caption: &'static str,
	on_input: impl Fn(String) -> Message + 'a,
	on_submit: Message,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let input = controls::text_input(placeholder, value)
		.on_input(on_input)
		.on_submit(on_submit.clone())
		.size(size)
		.width(Fill);
	let add = button(text(caption).size(size).center())
		.on_press(on_submit)
		.style(controls::secondary);
	row![input, add]
		.spacing(6)
		.align_y(Alignment::Center)
		.into()
}

/// An item offered in a picker.
#[derive(Debug, Clone, PartialEq)]
struct Choice<T> {
	id: T,
	name: String,
}

impl<T> std::fmt::Display for Choice<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.name)
	}
}

fn area_choices(projects: &ProjectsSettings) -> Vec<Choice<u64>> {
	projects
		.areas
		.iter()
		.map(|area| Choice {
			id: area.id,
			name: area.name.clone(),
		})
		.collect()
}

// ---------------------------------------------------------------------------
// The main window.

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	// Keep the pages visible when the panel has its own side window. Embedded
	// hosts without a main window still need somewhere to show the details.
	if state.editor_window.is_none() && state.life_panel_open() {
		return panel(state, metrics);
	}

	let page = state.life.page;
	let subtitle = match page {
		Page::Focus => "What this week is about, and what needs you",
		Page::Areas => "The parts of your life, and how each is going",
		Page::Goals => "What you are aiming for, and the objectives that get you there",
		Page::Timeline => "Objectives by when they are due",
		Page::Review => "Look back on the week and set up the next",
	};
	let tabs = segmented(
		&Page::ALL,
		page,
		Page::label,
		|page| msg(Action::ShowPage(page)),
		metrics.label() + 1.0,
	);

	let body = match page {
		Page::Focus => focus_page(state, metrics),
		Page::Areas => areas_page(state, metrics),
		Page::Goals => goals_page(state, metrics),
		Page::Timeline => timeline_page(state, metrics),
		Page::Review => review_page(state, metrics),
	};

	container(
		column![
			controls::page_heading("Life", subtitle, None, metrics),
			tabs,
			controls::scrollable(container(body).padding(Padding {
				right: SCROLLBAR_GUTTER,
				..Padding::ZERO
			}))
			.width(Fill)
			.height(Fill),
		]
		.spacing(metrics.spacing)
		.height(Fill),
	)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.into()
}

/// Why an open objective needs attention, if it does.
fn attention(objective: &Objective, pace: Pace, today: NaiveDate) -> Option<String> {
	if !objective.status.is_open() {
		return None;
	}
	match pace {
		Pace::Overdue => {
			return Some("Past its due date: finish it, move the date, or drop it".into());
		}
		Pace::Behind => return Some("Behind where the time gone by says it should be".into()),
		_ => {}
	}
	if objective.measure == Measure::Number {
		let stale = objective
			.last_check_in()
			.is_none_or(|day| (today - day).num_days() >= STALE_CHECK_IN_DAYS);
		if stale {
			return Some("No check-in for two weeks".into());
		}
	}
	None
}

fn focus_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let projects = &state.config.projects;
	let today = crate::life::today();
	let score = score_of(state);

	let open: Vec<(&LifeArea, &Objective)> = projects
		.all_objectives()
		.filter(|(_, o)| o.status.is_open())
		.collect();
	let paces: Vec<Pace> = open.iter().map(|(_, o)| o.pace(today, &score)).collect();
	let on_track = paces.iter().filter(|p| **p == Pace::OnTrack).count();
	let alarming = paces.iter().filter(|p| p.is_alarming()).count();
	let achieved = projects
		.all_objectives()
		.filter(|(_, o)| o.status == Status::Done)
		.count();

	let tiles = [
		stat(open.len().to_string(), "Active objectives", metrics),
		stat(on_track.to_string(), "On track", metrics),
		stat(alarming.to_string(), "Behind or overdue", metrics),
		stat(achieved.to_string(), "Achieved", metrics),
	];
	// Four across when there is room, otherwise two by two.
	let per_row = if metrics.size.width >= 380.0 { 4 } else { 2 };
	let mut stats = column![].spacing(metrics.spacing);
	let mut line = row![].spacing(metrics.spacing);
	for (index, tile) in tiles.into_iter().enumerate() {
		line = line.push(tile);
		if (index + 1) % per_row == 0 {
			stats = stats.push(line);
			line = row![].spacing(metrics.spacing);
		}
	}

	let mut page = column![stats].spacing(metrics.spacing).width(Fill);

	// This week's focus, like a weekly objective in a planner.
	let focus: Vec<_> = open.iter().filter(|(_, o)| o.focus).collect();
	page = page.push(gap());
	page = page.push(heading("This week's focus", Some(focus.len()), metrics));
	if focus.is_empty() {
		page = page.push(empty_state(
			"Nothing to focus on yet",
			"Pick a few objectives for the week in the Review page, or open one and turn on “Focus this week”.",
			metrics,
		));
	}
	for (area, objective) in &focus {
		page = page.push(objective_card(state, area, objective, None, metrics));
	}

	// The habits that carry out what is in focus, and how the week went.
	let mut habits: Vec<&crate::habits::Habit> = focus
		.iter()
		.flat_map(|(_, o)| o.habits.iter())
		.filter_map(|id| state.habits.library.habit(id))
		.filter(|habit| !habit.archived)
		.collect();
	habits.dedup_by(|a, b| a.id == b.id);
	if !habits.is_empty() {
		page = page.push(gap());
		page = page.push(heading(
			"Habits behind your focus",
			Some(habits.len()),
			metrics,
		));
		let mut list = column![].spacing(6);
		for habit in habits {
			let strength = habit_score(state, &habit.id)
				.map(percent)
				.unwrap_or_default();
			list = list.push(
				container(
					row![
						habit_dot(habit.color),
						text(&habit.name)
							.size(metrics.label())
							.wrapping(text::Wrapping::WordOrGlyph)
							.width(Fill),
						week_dots(state, habit),
						label(strength, metrics).width(Length::Fixed(40.0)),
					]
					.spacing(8)
					.align_y(Alignment::Center),
				)
				.padding([6, 10])
				.width(Fill)
				.style(controls::row_surface),
			);
		}
		page = page.push(list);
	}

	// What has slipped or gone quiet.
	let needs: Vec<_> = open
		.iter()
		.zip(&paces)
		.filter(|((_, o), _)| !o.focus)
		.filter_map(|((area, o), pace)| attention(o, *pace, today).map(|why| (*area, *o, why)))
		.collect();
	if !needs.is_empty() {
		page = page.push(gap());
		page = page.push(heading("Needs attention", Some(needs.len()), metrics));
		for (area, objective, why) in needs {
			page = page.push(objective_card(state, area, objective, Some(why), metrics));
		}
	}

	// What is coming up.
	let mut soon: Vec<_> = open
		.iter()
		.zip(&paces)
		.filter(|((_, o), pace)| !o.focus && !pace.is_alarming())
		.filter(|((_, o), _)| {
			o.due
				.is_some_and(|due| (0..=DUE_SOON_DAYS).contains(&(due - today).num_days()))
		})
		.map(|((area, o), _)| (*area, *o))
		.collect();
	soon.sort_by_key(|(_, o)| o.due);
	if !soon.is_empty() {
		page = page.push(gap());
		page = page.push(heading(
			"Due in the next 30 days",
			Some(soon.len()),
			metrics,
		));
		for (area, objective) in soon {
			page = page.push(objective_card(state, area, objective, None, metrics));
		}
	}

	if open.is_empty() {
		page = page.push(empty_state(
			"Start with a goal",
			"Open Goals to write down something you want, then add the objectives that will get you there.",
			metrics,
		));
	}

	page.into()
}

fn areas_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let projects = &state.config.projects;
	let size = metrics.label();

	let input = controls::text_input("New life area, e.g. Health", &state.life.new_name)
		.on_input(|name| msg(Action::NewName(name)))
		.on_submit(msg(Action::Add))
		.size(size)
		.width(Fill);
	let add = button(text("Add area").size(size).center())
		.on_press(msg(Action::Add))
		.style(controls::primary);
	let mut page = column![row![input, add].spacing(6).align_y(Alignment::Center)]
		.spacing(metrics.spacing)
		.width(Fill);
	if let Some(error) = &state.life.add_error {
		page = page.push(text(error).size(size).style(text::danger));
	}

	let count = projects.areas.len();
	for (index, area) in projects.areas.iter().enumerate() {
		let card = area_card(state, area, metrics);
		let shift = |glyph: &'static str, up: bool, enabled: bool| {
			button(text(glyph).size(size))
				.padding([0, 6])
				.on_press_maybe(enabled.then(|| msg(Action::ShiftArea(area.id, up))))
				.style(controls::ghost)
		};
		page = page.push(
			row![
				card,
				column![
					shift("↑", true, index > 0),
					shift("↓", false, index + 1 < count)
				]
				.spacing(2),
			]
			.spacing(4)
			.align_y(Alignment::Center),
		);
	}

	// Life balance, from the latest satisfaction ratings.
	page = page.push(gap());
	page = page.push(heading("Balance", None, metrics));
	let rated: Vec<_> = projects
		.areas
		.iter()
		.filter_map(|area| area.satisfaction().map(|r| (area, r.score)))
		.collect();
	if rated.is_empty() {
		page = page.push(empty_state(
			"No ratings yet",
			"Rate how satisfied you are with each area in the weekly review to see your balance here.",
			metrics,
		));
	} else {
		let mut wheel = column![].spacing(6);
		for (area, score) in rated {
			wheel = wheel.push(
				row![
					text(&area.name)
						.size(size)
						.width(Length::FillPortion(2))
						.wrapping(text::Wrapping::WordOrGlyph),
					container(bar(Some(f64::from(score) / 10.0), area_color(area), 10.0))
						.width(Length::FillPortion(5)),
					text(format!("{score}/10"))
						.size(size - 1.0)
						.width(Length::Fixed(40.0)),
				]
				.spacing(8)
				.align_y(Alignment::Center),
			);
		}
		page = page.push(
			container(wheel)
				.padding([10, 12])
				.width(Fill)
				.style(controls::row_surface),
		);
	}

	page.into()
}

/// How satisfaction has moved since the rating before the latest.
fn trend(area: &LifeArea) -> &'static str {
	let n = area.ratings.len();
	if n < 2 {
		return "";
	}
	match area.ratings[n - 1].score.cmp(&area.ratings[n - 2].score) {
		std::cmp::Ordering::Greater => " ↑",
		std::cmp::Ordering::Less => " ↓",
		std::cmp::Ordering::Equal => "",
	}
}

fn area_card<'a>(state: &'a State, area: &'a LifeArea, metrics: &Metrics) -> Element<'a, Message> {
	let size = metrics.label();
	let projects = &state.config.projects;
	let selected = state
		.life
		.open
		.as_ref()
		.is_some_and(|open| open.selection == Selection::Area(area.id));
	let project_count = projects
		.items
		.iter()
		.filter(|project| projects.resolve_area(project.area) == area.id)
		.count();
	let habit_count = state
		.habits
		.library
		.habits
		.iter()
		.filter(|habit| !habit.archived && projects.resolve_area(habit.area) == area.id)
		.count();
	let active_goals = area.goals.iter().filter(|g| g.status.is_open()).count();
	let active_objectives = area
		.objectives
		.iter()
		.filter(|o| o.status.is_open())
		.count();

	let mut top = row![
		color_circle(area_color(area)),
		text(&area.name)
			.wrapping(text::Wrapping::WordOrGlyph)
			.size(size + 2.0)
			.width(Fill),
	]
	.spacing(8)
	.align_y(Alignment::Center);
	if let Some(rating) = area.satisfaction() {
		top = top.push(chip(
			format!("{}/10{}", rating.score, trend(area)),
			if rating.score >= 7 {
				Tone::Good
			} else if rating.score >= 4 {
				Tone::Warn
			} else {
				Tone::Bad
			},
			size,
		));
	}

	let mut content = column![top].spacing(5).width(Fill);
	// The vision's first line says what the area is about.
	if let Some(line) = area
		.vision
		.lines()
		.map(|line| line.trim_start_matches(['#', '-', '*', ' ']).trim())
		.find(|line| !line.is_empty())
	{
		content = content.push(label(line.to_owned(), metrics));
	}
	let progress = area.progress(&score_of(state));
	if progress.is_some() {
		content = content.push(progress_line(progress, area_color(area), metrics));
	}
	content = content.push(
		label(
			[
				plural(active_goals, "goal", "goals"),
				plural(active_objectives, "objective", "objectives"),
				plural(project_count, "project", "projects"),
				plural(habit_count, "habit", "habits"),
			]
			.join(" · "),
			metrics,
		)
		.size(size - 2.0),
	);

	button(content)
		.on_press(open_msg(Selection::Area(area.id)))
		.padding([10, 12])
		.width(Fill)
		.style(if selected {
			controls::subtle_primary
		} else {
			controls::secondary
		})
		.into()
}

fn goals_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let projects = &state.config.projects;
	let size = metrics.label();
	let filter = state.life.goal_filter;
	let score = score_of(state);

	// Add a goal to any area.
	let options = area_choices(projects);
	let area = projects.resolve_area(state.life.new_goal_area);
	let current = options.iter().find(|c| c.id == area).cloned();
	let input = controls::text_input("New goal, e.g. Run a marathon", &state.life.new_goal)
		.on_input(|name| msg(Action::QuickGoal(name)))
		.on_submit(msg(Action::AddQuickGoal))
		.size(size)
		.width(Fill);
	let picker = controls::pick_list(options, current, |to: Choice<u64>| {
		msg(Action::QuickGoalArea(to.id))
	})
	.text_size(size - 1.0);
	let add = button(text("Add goal").size(size).center())
		.on_press(msg(Action::AddQuickGoal))
		.style(controls::primary);

	let mut page = column![
		row![input, picker, add]
			.spacing(6)
			.align_y(Alignment::Center),
		segmented(
			&GoalFilter::ALL,
			filter,
			GoalFilter::label,
			|f| msg(Action::GoalFilter(f)),
			size,
		),
	]
	.spacing(metrics.spacing)
	.width(Fill);

	let mut any = false;
	for area in &projects.areas {
		let goals: Vec<_> = area
			.goals
			.iter()
			.filter(|goal| filter.shows(goal.status))
			.collect();
		let loose: Vec<_> = area
			.objectives_of(None)
			.filter(|o| filter.shows(o.status))
			.collect();
		if goals.is_empty() && loose.is_empty() {
			continue;
		}
		any = true;
		page = page.push(gap());
		page = page.push(
			button(
				row![
					color_circle(area_color(area)),
					controls::section_heading(area.name.clone(), Some(goals.len()), size + 2.0),
				]
				.spacing(8)
				.align_y(Alignment::Center),
			)
			.on_press(open_msg(Selection::Area(area.id)))
			.padding([2, 0])
			.style(controls::ghost),
		);

		for goal in goals {
			let progress = area.goal_progress(goal, &score);
			let objectives: Vec<_> = area
				.objectives_of(Some(goal.id))
				.filter(|o| filter == GoalFilter::All || o.status != Status::Dropped)
				.collect();
			let mut header = column![
				row![
					text(&goal.name)
						.size(size + 2.0)
						.wrapping(text::Wrapping::WordOrGlyph)
						.width(Fill),
					chip(goal.status.label(), status_tone(goal.status), size),
				]
				.spacing(8)
				.align_y(Alignment::Center)
			]
			.spacing(4)
			.width(Fill);
			if !goal.why.trim().is_empty() {
				header = header.push(label(format!("Why: {}", goal.why), metrics));
			}
			if progress.is_some() {
				header = header.push(progress_line(progress, area_color(area), metrics));
			}
			let mut card = column![
				button(header)
					.on_press(open_msg(Selection::Goal(goal.id)))
					.padding([8, 10])
					.width(Fill)
					.style(controls::ghost)
			]
			.spacing(6)
			.width(Fill);
			for objective in &objectives {
				card = card.push(objective_row(state, area, objective, metrics));
			}
			if objectives.is_empty() {
				card = card.push(
					container(label(
						"No objectives yet. Open the goal to say how you will reach it.",
						metrics,
					))
					.padding([0, 10]),
				);
			}
			page = page.push(
				container(card)
					.padding(6)
					.width(Fill)
					.style(controls::row_surface),
			);
		}

		if !loose.is_empty() {
			let mut card =
				column![container(label("Objectives without a goal", metrics)).padding([2, 10])]
					.spacing(6)
					.width(Fill);
			for objective in loose {
				card = card.push(objective_row(state, area, objective, metrics));
			}
			page = page.push(
				container(card)
					.padding(6)
					.width(Fill)
					.style(controls::row_surface),
			);
		}
	}
	if !any {
		page = page.push(empty_state(
			match filter {
				GoalFilter::Done => "Nothing achieved yet",
				_ => "No goals yet",
			},
			"A goal is something you want, which could be reached in more than one way. Add one above.",
			metrics,
		));
	}
	page.into()
}

fn timeline_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let projects = &state.config.projects;
	let today = crate::life::today();
	let week_end = Horizon::Week.end(today, crate::life::week_start(state, today));
	let month_end = Horizon::Month.end(today, today);
	let quarter_out = today + chrono::Days::new(92);

	let mut objectives: Vec<_> = projects
		.all_objectives()
		.filter(|(_, o)| o.status.is_open() || o.status == Status::Paused)
		.collect();
	objectives.sort_by_key(|(_, o)| (o.due.is_none(), o.due));

	// Which group a due date falls in, by its index in `titles`.
	let titles = [
		"Overdue",
		"This week",
		"This month",
		"Next three months",
		"Later",
		"No due date",
	];
	let bucket = |due: Option<NaiveDate>| match due {
		None => 5,
		Some(d) if d < today => 0,
		Some(d) if d <= week_end => 1,
		Some(d) if d <= month_end => 2,
		Some(d) if d <= quarter_out => 3,
		Some(_) => 4,
	};

	let mut page = column![].spacing(metrics.spacing).width(Fill);
	for (index, title) in titles.into_iter().enumerate() {
		let items: Vec<_> = objectives
			.iter()
			.filter(|(_, o)| bucket(o.due) == index)
			.collect();
		if items.is_empty() {
			continue;
		}
		page = page.push(heading(title, Some(items.len()), metrics));
		for (area, objective) in items {
			let date: Element<'a, Message> = match objective.due {
				Some(due) => column![
					text(due.format("%b").to_string()).size(metrics.label() - 2.0),
					text(due.format("%-d").to_string()).size(metrics.label() + 6.0),
				]
				.align_x(Alignment::Center)
				.width(Length::Fixed(40.0))
				.into(),
				None => Space::new().width(Length::Fixed(40.0)).into(),
			};
			page = page.push(
				row![date, objective_card(state, area, objective, None, metrics)]
					.spacing(8)
					.align_y(Alignment::Center),
			);
		}
		page = page.push(gap());
	}
	if objectives.is_empty() {
		page = page.push(empty_state(
			"Nothing scheduled",
			"Give objectives a due date to see them laid out here.",
			metrics,
		));
	}
	page.into()
}

fn review_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let projects = &state.config.projects;
	let size = metrics.label();
	let week = state
		.life
		.review
		.week
		.unwrap_or_else(|| crate::life::review_week(state));
	let review = projects.review(week);
	let finished = review.is_some_and(|r| r.done);
	let today = crate::life::today();

	let title = format!(
		"Week of {} – {}",
		week.format("%b %-d"),
		(week + chrono::Days::new(6)).format("%b %-d")
	);
	let finish = button(
		text(if finished {
			"Review done ✓"
		} else {
			"Finish review"
		})
		.size(size),
	)
	.on_press(msg(Action::FinishReview))
	.style(if finished {
		controls::secondary
	} else {
		controls::primary
	});
	let mut page = column![
		row![
			text(title).size(size + 3.0).width(Fill),
			finish
		]
		.spacing(8)
		.align_y(Alignment::Center),
		label(
			"Take ten minutes: rate each area, check in on what you are working towards, pick next week's focus, and write down what you learned.",
			metrics,
		),
	]
	.spacing(metrics.spacing)
	.width(Fill);

	// 1. Satisfaction, area by area.
	page = page.push(gap());
	page = page.push(heading("1. How is each area going?", None, metrics));
	for area in &projects.areas {
		let current = area.rating_for(week);
		let previous = area
			.ratings
			.iter()
			.rev()
			.find(|rating| rating.date < week)
			.map(|rating| format!("Last time {}", rating.score))
			.unwrap_or_default();
		let mut scores = row![].spacing(3);
		for score in 1..=10u8 {
			scores = scores.push(
				button(text(score.to_string()).size(size - 1.0).center())
					.width(Length::Fixed(26.0))
					.padding([3, 0])
					.on_press(msg(Action::Rate(area.id, score)))
					.style(controls::nav(current == Some(score))),
			);
		}
		page = page.push(
			container(
				column![
					row![
						color_circle(area_color(area)),
						text(&area.name).size(size).width(Fill),
						label(previous, metrics),
					]
					.spacing(8)
					.align_y(Alignment::Center),
					scores.wrap().vertical_spacing(3.0),
				]
				.spacing(6),
			)
			.padding([8, 10])
			.width(Fill)
			.style(controls::row_surface),
		);
	}

	// 2. Check in on what is open.
	let open: Vec<_> = projects
		.all_objectives()
		.filter(|(_, o)| o.status.is_open())
		.collect();
	page = page.push(gap());
	page = page.push(heading(
		"2. Check in on your objectives",
		Some(open.len()),
		metrics,
	));
	if open.is_empty() {
		page = page.push(label("No active objectives.", metrics));
	}
	for (area, objective) in &open {
		page = page.push(review_check_in(state, area, objective, today, metrics));
	}

	// 3. Next week's focus.
	page = page.push(gap());
	page = page.push(heading("3. Choose what to focus on", None, metrics));
	page = page.push(label(
		"Three to five objectives is plenty. They are shown first on the Focus page.",
		metrics,
	));
	let mut picks = column![].spacing(4);
	for (area, objective) in &open {
		let id = objective.id;
		picks = picks.push(
			controls::checkbox(objective.focus)
				.label(format!("{}  ·  {}", objective.name, area.name))
				.text_size(size)
				.on_toggle(move |on| msg(Action::Focus(id, on))),
		);
	}
	page = page.push(picks);

	// 4. Reflection.
	page = page.push(gap());
	page = page.push(heading("4. Reflect", None, metrics));
	page = page.push(
		controls::text_editor(&state.life.review.reflection)
			.placeholder("What went well? What didn't? What will you do differently next week?")
			.on_action(|action| msg(Action::ReviewEdited(action)))
			.size(size + 1.0)
			.height(Length::Fixed((metrics.size.height * 0.25).max(120.0))),
	);

	// Earlier reviews.
	let earlier: Vec<_> = projects
		.reviews
		.iter()
		.rev()
		.filter(|r| r.date < week && !r.reflection.trim().is_empty())
		.take(6)
		.collect();
	if !earlier.is_empty() {
		page = page.push(gap());
		page = page.push(heading("Earlier reviews", Some(earlier.len()), metrics));
		for review in earlier {
			page = page.push(
				column![
					label(
						format!("Week of {}", review.date.format("%b %-d, %Y")),
						metrics
					),
					container(super::notes::rendered(&review.reflection, size))
						.padding(10.0)
						.width(Fill)
						.style(super::notes::note_surface(Pastel::default())),
				]
				.spacing(4),
			);
		}
	}

	page.into()
}

/// One objective in the review, with what checking in on it takes.
fn review_check_in<'a>(
	state: &'a State,
	area: &'a LifeArea,
	objective: &'a Objective,
	today: NaiveDate,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let score = score_of(state);
	let progress = objective.progress(&score);
	let pace = objective.pace(today, &score);

	let mut content = column![
		row![
			color_circle(area_color(area)),
			button(
				text(&objective.name)
					.size(size)
					.wrapping(text::Wrapping::WordOrGlyph)
			)
			.padding(0)
			.on_press(open_msg(Selection::Objective(id)))
			.style(controls::ghost)
			.width(Fill),
			pace_chip(pace, size),
		]
		.spacing(8)
		.align_y(Alignment::Center),
	]
	.spacing(6)
	.width(Fill);
	if objective.measure != Measure::None {
		content = content.push(progress_line(progress, area_color(area), metrics));
	}

	let control: Element<'a, Message> = match objective.measure {
		Measure::Number => {
			let typed = state
				.life
				.review
				.check_ins
				.get(&id)
				.map(String::as_str)
				.unwrap_or_default();
			let placeholder = match objective.current() {
				Some(current) => format!(
					"Now {} {}",
					crate::life::format_number(current),
					objective.unit
				),
				None => "Current value".into(),
			};
			add_row(
				placeholder.trim(),
				typed,
				"Check in",
				move |value| msg(Action::ReviewCheckInValue(id, value)),
				msg(Action::ReviewCheckIn(id)),
				metrics,
			)
		}
		Measure::Milestones => {
			let mut list = column![].spacing(3);
			for milestone in &objective.milestones {
				let m = milestone.id;
				list = list.push(
					controls::checkbox(milestone.done)
						.label(milestone.name.as_str())
						.text_size(size)
						.on_toggle(move |_| msg(Action::ToggleMilestone(id, m))),
				);
			}
			if objective.milestones.is_empty() {
				list = list.push(label("No milestones yet. Open it to add some.", metrics));
			}
			list.into()
		}
		Measure::Habits => label(
			measure_detail(state, objective).unwrap_or_default(),
			metrics,
		)
		.into(),
		Measure::None => button(text("Mark done").size(size - 1.0))
			.on_press(msg(Action::ObjectiveStatus(id, Status::Done)))
			.style(controls::secondary)
			.into(),
	};
	content = content.push(control);

	container(content)
		.padding([8, 10])
		.width(Fill)
		.style(controls::row_surface)
		.into()
}

// ---------------------------------------------------------------------------
// The side panel.

pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let Some(open) = &state.life.open else {
		return Space::new().into();
	};
	let size = metrics.label();
	let projects = &state.config.projects;

	// Where it sits, each step of which can be opened.
	let mut crumbs = row![].spacing(2).align_y(Alignment::Center);
	let crumb = |caption: String, to: Selection| {
		button(text(caption).size(size - 1.0).style(text::secondary))
			.padding([2, 4])
			.on_press(open_msg(to))
			.style(controls::ghost)
	};
	let separator = || text("›").size(size - 1.0).style(text::secondary);
	match open.selection {
		Selection::Goal(id) => {
			if let Some((area, _)) = projects.find_goal(id) {
				crumbs = crumbs.push(crumb(area.name.clone(), Selection::Area(area.id)));
			}
		}
		Selection::Objective(id) => {
			if let Some((area, objective)) = projects.find_objective(id) {
				crumbs = crumbs.push(crumb(area.name.clone(), Selection::Area(area.id)));
				if let Some(goal) = area
					.objective_goal(objective)
					.and_then(|goal| area.goal(goal))
				{
					crumbs = crumbs.push(separator());
					crumbs = crumbs.push(crumb(goal.name.clone(), Selection::Goal(goal.id)));
				}
			}
		}
		Selection::Project(id) => {
			if let Some(project) = projects.get(id) {
				let area = projects.resolve_area(project.area);
				if let Some(area) = projects.area(area) {
					crumbs = crumbs.push(crumb(area.name.clone(), Selection::Area(area.id)));
				}
			}
		}
		Selection::Area(_) => {}
	}

	let mut top = row![].spacing(6).align_y(Alignment::Center);
	if !open.back.is_empty() {
		top = top.push(
			button(text("‹ Back").size(size))
				.on_press(msg(Action::Back))
				.style(controls::ghost),
		);
	}
	top = top.push(container(crumbs.wrap()).width(Fill));
	top = top.push(
		button(text("Close").size(size))
			.on_press(msg(Action::Close))
			.style(controls::ghost),
	);

	let body = match open.selection {
		Selection::Area(id) => projects
			.area(id)
			.map(|area| area_panel(state, area, open, metrics)),
		Selection::Goal(id) => projects
			.find_goal(id)
			.map(|(area, goal)| goal_panel(state, area, goal, open, metrics)),
		Selection::Objective(id) => projects
			.find_objective(id)
			.map(|(area, objective)| objective_panel(state, area, objective, open, metrics)),
		Selection::Project(id) => projects
			.get(id)
			.map(|project| project_panel(state, project, open, metrics)),
	}
	.unwrap_or_else(|| Space::new().into());

	container(
		column![
			top,
			controls::scrollable(container(body).padding(Padding {
				right: SCROLLBAR_GUTTER,
				..Padding::ZERO
			}))
			.width(Fill)
			.height(Fill),
		]
		.spacing(metrics.spacing)
		.width(Fill)
		.height(Fill),
	)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.into()
}

/// The kind of thing open, over its name.
fn panel_title<'a>(
	kind: &'static str,
	color: Color,
	title: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	column![
		row![
			color_circle(color),
			text(kind.to_uppercase())
				.size(metrics.label() - 2.0)
				.style(text::secondary)
		]
		.spacing(6)
		.align_y(Alignment::Center),
		title,
	]
	.spacing(4)
	.width(Fill)
	.into()
}

fn area_panel<'a>(
	state: &'a State,
	area: &'a LifeArea,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let projects = &state.config.projects;
	let score = score_of(state);

	let mut page = column![panel_title(
		"Life area",
		area_color(area),
		text(&area.name)
			.size(size + 8.0)
			.wrapping(text::Wrapping::WordOrGlyph)
			.into(),
		metrics,
	)]
	.spacing(metrics.spacing)
	.width(Fill);

	// Satisfaction this week.
	let week = crate::life::review_week(state);
	let current = area.rating_for(week);
	let mut scores = row![].spacing(3);
	for value in 1..=10u8 {
		scores = scores.push(
			button(text(value.to_string()).size(size - 1.0).center())
				.width(Length::Fixed(26.0))
				.padding([3, 0])
				.on_press(msg(Action::Rate(area.id, value)))
				.style(controls::nav(current == Some(value))),
		);
	}
	let history: String = area
		.ratings
		.iter()
		.rev()
		.take(8)
		.rev()
		.map(|r| r.score.to_string())
		.collect::<Vec<_>>()
		.join(" · ");
	let mut satisfaction = column![
		label("How satisfied are you with this area this week?", metrics),
		scores.wrap().vertical_spacing(3.0),
	]
	.spacing(4);
	if !history.is_empty() {
		satisfaction = satisfaction.push(label(format!("Recent weeks: {history}"), metrics));
	}
	page = page.push(satisfaction);
	if let Some(progress) = area.progress(&score) {
		page = page.push(
			column![
				label("Objective progress", metrics),
				progress_line(Some(progress), area_color(area), metrics)
			]
			.spacing(4),
		);
	}

	// Vision.
	let mode = |caption: &'static str, which: VisionMode| {
		button(text(caption).size(size - 1.0))
			.on_press(msg(Action::VisionMode(which)))
			.style(controls::nav(open.vision_mode == which))
	};
	let vision: Element<'a, Message> = match open.vision_mode {
		VisionMode::Edit => controls::text_editor(&open.vision)
			.placeholder(
				"Picture this part of your life a few years from now. Markdown works: # headings, - bullets, **bold**",
			)
			.on_action(|action| msg(Action::VisionEdited(action)))
			.size(size + 1.0)
			.height(Length::Fixed((metrics.size.height * 0.3).max(140.0)))
			.into(),
		VisionMode::View if area.vision.trim().is_empty() => {
			label("No vision written yet. Choose Edit to write one.", metrics).into()
		}
		VisionMode::View => container(super::notes::rendered(&area.vision, size + 1.0))
			.padding(10.0)
			.width(Fill)
			.style(super::notes::note_surface(Pastel::default()))
			.into(),
	};
	page = page.push(gap());
	page = page.push(
		row![
			container(heading("Vision", None, metrics)).width(Fill),
			mode("Edit", VisionMode::Edit),
			mode("View", VisionMode::View),
		]
		.spacing(4)
		.align_y(Alignment::Center),
	);
	page = page.push(vision);

	// Goals.
	page = page.push(gap());
	page = page.push(heading("Goals", Some(area.goals.len()), metrics));
	for goal in &area.goals {
		let progress = area.goal_progress(goal, &score);
		let count = area.objectives_of(Some(goal.id)).count();
		let mut words = column![
			row![
				text(&goal.name)
					.size(size + 1.0)
					.wrapping(text::Wrapping::WordOrGlyph)
					.width(Fill),
				chip(goal.status.label(), status_tone(goal.status), size),
			]
			.spacing(8)
			.align_y(Alignment::Center),
		]
		.spacing(4)
		.width(Fill);
		if !goal.why.trim().is_empty() {
			words = words.push(label(goal.why.as_str(), metrics));
		}
		if progress.is_some() {
			words = words.push(progress_line(progress, area_color(area), metrics));
		}
		words = words.push(label(plural(count, "objective", "objectives"), metrics));
		page = page.push(
			button(words)
				.on_press(open_msg(Selection::Goal(goal.id)))
				.padding([8, 10])
				.width(Fill)
				.style(controls::secondary),
		);
	}
	if area.goals.is_empty() {
		page = page.push(label(
			"What do you want in this part of your life? Goals can be big and open-ended.",
			metrics,
		));
	}
	page = page.push(add_row(
		"New goal",
		&open.new_goal,
		"Add goal",
		|name| msg(Action::NewGoal(name)),
		msg(Action::AddGoal),
		metrics,
	));

	// Objectives that serve no goal.
	let loose: Vec<_> = area.objectives_of(None).collect();
	page = page.push(gap());
	page = page.push(heading(
		"Objectives without a goal",
		Some(loose.len()),
		metrics,
	));
	for objective in loose {
		page = page.push(objective_row(state, area, objective, metrics));
	}
	page = page.push(add_row(
		"New objective",
		&open.new_objective,
		"Add objective",
		|name| msg(Action::NewObjective(name)),
		msg(Action::AddObjective),
		metrics,
	));

	// Projects.
	page = page.push(gap());
	let in_area: Vec<_> = projects
		.sorted()
		.into_iter()
		.filter(|project| projects.resolve_area(project.area) == area.id)
		.collect();
	page = page.push(heading("Projects", Some(in_area.len()), metrics));
	for project in in_area {
		let tasks = project_tasks(state, project.id).len();
		let serves = projects.objectives_of_project(project.id).count();
		let mut detail = vec![plural(tasks, "open task", "open tasks")];
		if serves > 0 {
			detail.push(format!(
				"serves {}",
				plural(serves, "objective", "objectives")
			));
		}
		page = page.push(
			button(
				row![
					color_circle(project.color().unwrap_or(NO_COLOR)),
					column![
						text(&project.name)
							.size(size)
							.wrapping(text::Wrapping::WordOrGlyph),
						label(detail.join(" · "), metrics),
					]
					.spacing(2)
					.width(Fill),
					text("›").size(size + 2.0),
				]
				.spacing(8)
				.align_y(Alignment::Center),
			)
			.on_press(open_msg(Selection::Project(project.id)))
			.padding([6, 10])
			.width(Fill)
			.style(controls::secondary),
		);
	}
	page = page.push(add_row(
		"New project, e.g. School/Physics",
		&open.new_project,
		"Add project",
		|name| msg(Action::NewProject(name)),
		msg(Action::AddProject),
		metrics,
	));
	if let Some(error) = &open.new_project_error {
		page = page.push(text(error).size(size).style(text::danger));
	}

	// Habits filed here that carry out none of its objectives.
	page = page.push(gap());
	page = page.push(unassigned_habits(state, area, metrics));

	// Name, color and removal.
	page = page.push(gap());
	page = page.push(
		button(
			text(if open.settings_open {
				"⌄ Area settings"
			} else {
				"› Area settings"
			})
			.size(size),
		)
		.on_press(msg(Action::ToggleSettings))
		.style(controls::ghost),
	);
	if open.settings_open {
		let name = controls::text_input("Life area name", &open.name)
			.on_input(|name| msg(Action::Rename(name)))
			.size(size)
			.width(Fill);
		let mut settings = column![label("Name", metrics), name].spacing(4).width(Fill);
		if let Some(error) = &open.name_error {
			settings = settings.push(text(error).size(size).style(text::danger));
		}
		settings = settings.push(label("Color", metrics)).push(color_picker(
			area.color(),
			|c| msg(Action::Color(c)),
			metrics,
		));
		settings = settings.push(Space::new().height(Length::Fixed(6.0)));
		if area.id == crate::config::PERSONAL_AREA {
			settings = settings.push(label(
				"Projects and habits not in another area are kept here, so it cannot be removed.",
				metrics,
			));
		} else {
			settings = settings.push(remove_control(
				open,
				"life area",
				"Its projects and habits move to Personal, and its goals and objectives are removed.",
				metrics,
			));
		}
		page = page.push(settings);
	}

	page.into()
}

/// Habits in this area that no objective of it is carried out by.
fn unassigned_habits<'a>(
	state: &'a State,
	area: &'a LifeArea,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let projects = &state.config.projects;
	let options = area_choices(projects);
	let current = options.iter().find(|choice| choice.id == area.id).cloned();

	let habits: Vec<_> = state
		.habits
		.library
		.habits
		.iter()
		.filter(|habit| projects.resolve_area(habit.area) == area.id && !area.has_habit(&habit.id))
		.collect();
	let mut list = column![heading("Other habits", Some(habits.len()), metrics)].spacing(6);
	if habits.is_empty() {
		list = list.push(label("Every habit here carries out an objective.", metrics));
	}
	for habit in habits {
		let id = habit.id.clone();
		let picker = controls::pick_list(options.clone(), current.clone(), move |to| {
			msg(Action::MoveHabit(id.clone(), to.id))
		})
		.text_size(size - 1.0);
		let goal = habit.goal_label();
		let detail = if habit.archived {
			format!("{goal} · Archived")
		} else {
			goal
		};
		list = list.push(
			container(
				row![
					habit_dot(habit.color),
					column![
						text(&habit.name)
							.size(size)
							.wrapping(text::Wrapping::WordOrGlyph),
						label(detail, metrics),
					]
					.spacing(2)
					.width(Fill),
					picker,
				]
				.spacing(8)
				.align_y(Alignment::Center),
			)
			.padding([6, 8])
			.width(Fill)
			.style(controls::row_surface),
		);
	}
	list.into()
}

fn goal_panel<'a>(
	state: &'a State,
	area: &'a LifeArea,
	goal: &'a crate::config::Goal,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = goal.id;
	let score = score_of(state);

	let name = controls::text_input("Goal", &goal.name)
		.on_input(move |name| msg(Action::GoalName(id, name)))
		.size(size + 4.0)
		.width(Fill);
	let why = controls::text_input("Why does this matter to you?", &goal.why)
		.on_input(move |why| msg(Action::GoalWhy(id, why)))
		.size(size)
		.width(Fill);
	let description = controls::text_input("What reaching it looks like", &goal.description)
		.on_input(move |description| msg(Action::GoalDescription(id, description)))
		.size(size)
		.width(Fill);

	let mut page = column![
		panel_title("Goal", area_color(area), name.into(), metrics),
		segmented(
			&Status::ALL,
			goal.status,
			Status::label,
			move |status| msg(Action::GoalStatus(id, status)),
			size,
		),
		label("Why it matters", metrics),
		why,
		label("Description", metrics),
		description,
		label("Progress, from its objectives", metrics),
		progress_line(area.goal_progress(goal, &score), area_color(area), metrics),
	]
	.spacing(metrics.spacing)
	.width(Fill);

	let objectives: Vec<_> = area.objectives_of(Some(id)).collect();
	page = page.push(gap());
	page = page.push(heading("Objectives", Some(objectives.len()), metrics));
	page = page.push(label(
		"How will you reach it? Make each objective specific and measurable, with a date.",
		metrics,
	));
	for objective in objectives {
		page = page.push(objective_row(state, area, objective, metrics));
	}
	page = page.push(add_row(
		"New objective, e.g. Run 30 km a week by March",
		&open.new_objective,
		"Add objective",
		|name| msg(Action::NewObjective(name)),
		msg(Action::AddObjective),
		metrics,
	));

	page = page.push(gap());
	page = page.push(remove_control(
		open,
		"goal",
		"Its objectives stay, without a goal.",
		metrics,
	));
	page.into()
}

fn objective_panel<'a>(
	state: &'a State,
	area: &'a LifeArea,
	objective: &'a Objective,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let today = crate::life::today();
	let score = score_of(state);
	let progress = objective.progress(&score);
	let pace = objective.pace(today, &score);
	let time_format = state.config.appearance.time_format;

	let name = controls::text_input("Objective", &objective.name)
		.on_input(move |name| msg(Action::ObjectiveName(id, name)))
		.size(size + 4.0)
		.width(Fill);
	let description =
		controls::text_input("Details: what, how much, by when", &objective.description)
			.on_input(move |description| msg(Action::ObjectiveDescription(id, description)))
			.size(size)
			.width(Fill);

	// The goal it serves.
	let mut goal_options = vec![Choice {
		id: None,
		name: "No goal".to_owned(),
	}];
	goal_options.extend(area.goals.iter().map(|goal| Choice {
		id: Some(goal.id),
		name: goal.name.clone(),
	}));
	let current_goal = area.objective_goal(objective);
	let selected_goal = goal_options.iter().find(|c| c.id == current_goal).cloned();
	let goal_picker = controls::pick_list(
		goal_options,
		selected_goal,
		move |to: Choice<Option<u64>>| msg(Action::ObjectiveGoal(id, to.id)),
	)
	.text_size(size - 1.0);

	let focus = controls::toggler(objective.focus)
		.label("Focus this week")
		.text_size(size)
		.on_toggle_maybe(
			objective
				.status
				.is_open()
				.then_some(move |on| msg(Action::Focus(id, on))),
		);

	let mut summary = row![
		container(progress_line(progress, area_color(area), metrics)).width(Fill),
		pace_chip(pace, size),
	]
	.spacing(8)
	.align_y(Alignment::Center);
	if objective.measure == Measure::None && objective.status != Status::Done {
		summary = row![
			label("Not measured: it is done when you say so.", metrics).width(Fill),
			pace_chip(pace, size),
		]
		.spacing(8)
		.align_y(Alignment::Center);
	}

	let mut page = column![
		panel_title("Objective", area_color(area), name.into(), metrics),
		description,
		summary,
		row![label("Goal", metrics), goal_picker]
			.spacing(8)
			.align_y(Alignment::Center),
		segmented(
			&Status::ALL,
			objective.status,
			Status::label,
			move |status| msg(Action::ObjectiveStatus(id, status)),
			size,
		),
		focus,
	]
	.spacing(metrics.spacing)
	.width(Fill);

	// Time frame.
	let date_value = |date: Option<NaiveDate>| date.map(|d| d.to_string()).unwrap_or_default();
	let start = controls::date(
		time_format,
		&date_value(objective.start),
		true,
		false,
		true,
		size,
		move |value| msg(Action::Start(id, value)),
	);
	let due = controls::date(
		time_format,
		&date_value(objective.due),
		true,
		false,
		true,
		size,
		move |value| msg(Action::Due(id, value)),
	);
	let mut horizons = row![label("Due by end of", metrics)]
		.spacing(4)
		.align_y(Alignment::Center);
	for horizon in Horizon::ALL {
		horizons = horizons.push(
			button(text(horizon.label().trim_start_matches("This ").to_owned()).size(size - 1.0))
				.on_press(msg(Action::DueBy(id, horizon)))
				.style(controls::secondary),
		);
	}
	page = page.push(gap());
	page = page.push(heading("Time frame", None, metrics));
	page = page.push(
		row![
			column![label("Start", metrics), start]
				.spacing(4)
				.width(Fill),
			column![label("Due", metrics), due].spacing(4).width(Fill),
		]
		.spacing(8),
	);
	page = page.push(horizons.wrap().vertical_spacing(4.0));
	if let Some(due) = objective.due {
		page = page.push(label(due_label(due, today), metrics));
	}

	// How progress is measured.
	page = page.push(gap());
	page = page.push(heading("Measure", None, metrics));
	page = page.push(segmented(
		&Measure::ALL,
		objective.measure,
		Measure::label,
		move |measure| msg(Action::Measure(id, measure)),
		size,
	));
	match objective.measure {
		Measure::None => {}
		Measure::Number => page = page.push(number_editor(objective, open, metrics)),
		Measure::Milestones => page = page.push(milestone_editor(objective, open, metrics)),
		Measure::Habits => {
			page = page.push(label(
				"Progress is the average strength of the habits below.",
				metrics,
			));
		}
	}

	// Habits.
	page = page.push(gap());
	page = page.push(objective_habits(state, objective, metrics));

	// Projects.
	page = page.push(gap());
	page = page.push(objective_projects(state, objective, metrics));

	page = page.push(gap());
	page = page.push(remove_control(
		open,
		"objective",
		"Its check-ins and milestones go with it.",
		metrics,
	));
	page.into()
}

fn number_editor<'a>(
	objective: &'a Objective,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let field = |placeholder: &str, value: &str, on_input: fn(String) -> Action| {
		controls::text_input(placeholder, value)
			.on_input(move |text| msg(on_input(text)))
			.size(size)
			.width(Fill)
	};
	let unit = controls::text_input("kg, km/week, …", &objective.unit)
		.on_input(move |unit| msg(Action::Unit(id, unit)))
		.size(size)
		.width(Fill);

	let mut content = column![
		row![
			column![
				label("Start", metrics),
				field("e.g. 90", &open.baseline, Action::Baseline)
			]
			.spacing(4)
			.width(Fill),
			column![
				label("Target", metrics),
				field("e.g. 80", &open.target, Action::Target)
			]
			.spacing(4)
			.width(Fill),
			column![label("Unit", metrics), unit].spacing(4).width(Fill),
		]
		.spacing(8),
		label("Check in", metrics),
		row![
			controls::text_input("Value today", &open.check_in)
				.on_input(|value| msg(Action::CheckInValue(value)))
				.on_submit(msg(Action::CheckIn))
				.size(size)
				.width(Length::FillPortion(2)),
			controls::text_input("Note (optional)", &open.check_in_note)
				.on_input(|note| msg(Action::CheckInNote(note)))
				.on_submit(msg(Action::CheckIn))
				.size(size)
				.width(Length::FillPortion(3)),
			button(text("Log").size(size).center())
				.on_press(msg(Action::CheckIn))
				.style(controls::primary),
		]
		.spacing(6)
		.align_y(Alignment::Center),
	]
	.spacing(6)
	.width(Fill);

	if !objective.check_ins.is_empty() {
		content = content.push(label("History", metrics));
		let mut history = column![].spacing(2);
		for check_in in objective.check_ins.iter().rev().take(12) {
			let date = check_in.date;
			let mut line = row![
				text(date.format("%b %-d, %Y").to_string())
					.size(size - 1.0)
					.width(Length::Fixed(100.0)),
				text(format!(
					"{} {}",
					crate::life::format_number(check_in.value),
					objective.unit
				))
				.size(size - 1.0)
				.width(Length::Fixed(90.0)),
			]
			.spacing(6)
			.align_y(Alignment::Center);
			line = line.push(label(check_in.note.as_str(), metrics).width(Fill));
			line = line.push(
				button(text("×").size(size))
					.padding([0, 6])
					.on_press(msg(Action::RemoveCheckIn(id, date)))
					.style(controls::ghost),
			);
			history = history.push(line);
		}
		content = content.push(
			container(history)
				.padding([6, 10])
				.width(Fill)
				.style(controls::row_surface),
		);
	}
	content.into()
}

fn milestone_editor<'a>(
	objective: &'a Objective,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let mut list = column![].spacing(4).width(Fill);
	for milestone in &objective.milestones {
		let m = milestone.id;
		list = list.push(
			row![
				container(
					controls::checkbox(milestone.done)
						.label(milestone.name.as_str())
						.text_size(size)
						.on_toggle(move |_| msg(Action::ToggleMilestone(id, m)))
				)
				.width(Fill),
				button(text("×").size(size))
					.padding([0, 6])
					.on_press(msg(Action::RemoveMilestone(id, m)))
					.style(controls::ghost),
			]
			.spacing(6)
			.align_y(Alignment::Center),
		);
	}
	if objective.milestones.is_empty() {
		list = list.push(label("Break it into steps you can tick off.", metrics));
	}
	list.push(add_row(
		"New milestone",
		&open.new_milestone,
		"Add",
		|name| msg(Action::NewMilestone(name)),
		msg(Action::AddMilestone),
		metrics,
	))
	.into()
}

fn objective_habits<'a>(
	state: &'a State,
	objective: &'a Objective,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let mut list = column![heading("Habits", Some(objective.habits.len()), metrics)]
		.spacing(6)
		.width(Fill);

	for habit_id in &objective.habits {
		let Some(habit) = state.habits.library.habit(habit_id) else {
			continue;
		};
		let mut caption = habit.name.clone();
		if habit.archived {
			caption.push_str(" · Archived");
		}
		let strength = habit_score(state, habit_id)
			.map(percent)
			.unwrap_or_default();
		let habit_id = habit_id.clone();
		list = list.push(
			container(
				row![
					habit_dot(habit.color),
					text(caption)
						.size(size)
						.wrapping(text::Wrapping::WordOrGlyph)
						.width(Fill),
					week_dots(state, habit),
					label(strength, metrics).width(Length::Fixed(40.0)),
					button(text("×").size(size))
						.padding([0, 6])
						.on_press(msg(Action::RemoveHabit(id, habit_id)))
						.style(controls::ghost),
				]
				.spacing(8)
				.align_y(Alignment::Center),
			)
			.padding([6, 10])
			.width(Fill)
			.style(controls::row_surface),
		);
	}

	// Any existing habit not already here can be added; it moves out of
	// whatever objective it served before.
	let options: Vec<Choice<String>> = state
		.habits
		.library
		.habits
		.iter()
		.filter(|habit| !habit.archived && !objective.habits.contains(&habit.id))
		.map(|habit| Choice {
			id: habit.id.clone(),
			name: habit.name.clone(),
		})
		.collect();
	if options.is_empty() {
		list = list.push(label(
			if state.habits.library.habits.is_empty() {
				"No habits yet. Create them in Habits."
			} else {
				"Every habit is already here."
			},
			metrics,
		));
	} else {
		list = list.push(
			controls::pick_list(options, None::<Choice<String>>, move |habit| {
				msg(Action::AddHabit(id, habit.id))
			})
			.placeholder("Add a habit that carries it out…")
			.text_size(size - 1.0),
		);
	}
	list.into()
}

fn objective_projects<'a>(
	state: &'a State,
	objective: &'a Objective,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = objective.id;
	let projects = &state.config.projects;
	let mut list = column![heading("Projects", Some(objective.projects.len()), metrics)]
		.spacing(6)
		.width(Fill);
	for &project_id in &objective.projects {
		let Some(project) = projects.get(project_id) else {
			continue;
		};
		let tasks = project_tasks(state, project_id).len();
		list = list.push(
			row![
				button(
					row![
						color_circle(project.color().unwrap_or(NO_COLOR)),
						text(&project.name)
							.size(size)
							.wrapping(text::Wrapping::WordOrGlyph)
							.width(Fill),
						label(plural(tasks, "open task", "open tasks"), metrics),
					]
					.spacing(8)
					.align_y(Alignment::Center),
				)
				.on_press(open_msg(Selection::Project(project_id)))
				.padding([6, 10])
				.width(Fill)
				.style(controls::secondary),
				button(text("×").size(size))
					.padding([0, 6])
					.on_press(msg(Action::UnlinkProject(id, project_id)))
					.style(controls::ghost),
			]
			.spacing(4)
			.align_y(Alignment::Center),
		);
	}
	let options: Vec<Choice<u64>> = projects
		.sorted()
		.into_iter()
		.filter(|project| !objective.projects.contains(&project.id))
		.map(|project| Choice {
			id: project.id,
			name: project.name.clone(),
		})
		.collect();
	if !options.is_empty() {
		list = list.push(
			controls::pick_list(options, None::<Choice<u64>>, move |project| {
				msg(Action::LinkProject(id, project.id))
			})
			.placeholder("Link a project that carries it out…")
			.text_size(size - 1.0),
		);
	}
	list.into()
}

fn project_panel<'a>(
	state: &'a State,
	project: &'a crate::config::Project,
	open: &'a Open,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let size = metrics.label();
	let id = project.id;
	let projects = &state.config.projects;

	let name = controls::text_input("Project name", &open.name)
		.on_input(|name| msg(Action::Rename(name)))
		.size(size + 4.0)
		.width(Fill);
	let mut page = column![panel_title(
		"Project",
		project.color().unwrap_or(NO_COLOR),
		name.into(),
		metrics
	)]
	.spacing(metrics.spacing)
	.width(Fill);
	if let Some(error) = &open.name_error {
		page = page.push(text(error).size(size).style(text::danger));
	}

	// What it is for.
	let serves: Vec<_> = projects.objectives_of_project(id).collect();
	page = page.push(gap());
	page = page.push(heading("Objectives it serves", Some(serves.len()), metrics));
	if serves.is_empty() {
		page = page.push(label(
			"Link it from an objective's page to see how it moves your goals.",
			metrics,
		));
	}
	for (area, objective) in serves {
		page = page.push(objective_row(state, area, objective, metrics));
	}

	// Its open tasks.
	let tasks = project_tasks(state, id);
	page = page.push(gap());
	page = page.push(heading("Open tasks", Some(tasks.len()), metrics));
	if tasks.is_empty() {
		page = page.push(label("No open tasks filed under it.", metrics));
	}
	let mut list = column![].spacing(2);
	for task in tasks.iter().take(15) {
		list = list.push(
			row![
				text("○").size(size - 1.0).style(text::secondary),
				text(task.title.as_str())
					.size(size)
					.wrapping(text::Wrapping::WordOrGlyph)
					.width(Fill),
			]
			.spacing(8),
		);
	}
	if tasks.len() > 15 {
		list = list.push(label(format!("and {} more", tasks.len() - 15), metrics));
	}
	page = page.push(list);

	// Details.
	let options = area_choices(projects);
	let current = options
		.iter()
		.find(|choice| choice.id == projects.resolve_area(project.area))
		.cloned();
	let area_picker = controls::pick_list(options, current, move |to: Choice<u64>| {
		msg(Action::MoveProject(id, to.id))
	})
	.text_size(size - 1.0);
	let directory = controls::text_input("Directory, e.g. ~/Projects/site", &open.directory)
		.on_input(|directory| msg(Action::ProjectDirectory(directory)))
		.size(size)
		.width(Fill);
	page = page.push(gap());
	page = page.push(heading("Details", None, metrics));
	page = page.push(
		column![
			label("Life area", metrics),
			area_picker,
			label("Directory", metrics),
			directory,
			label("Color", metrics),
			color_picker(project.color(), |c| msg(Action::Color(c)), metrics),
		]
		.spacing(4),
	);
	page = page.push(gap());
	page = page.push(remove_control(
		open,
		"project",
		"Tasks filed under it lose their project.",
		metrics,
	));
	page.into()
}
