//! Habits: goals tracked day by day, the way Loop Habit Tracker does.
//!
//! A habit ([`timeparagon_core::habits::Habit`]) lives in a local file and needs
//! nothing else. Separately, a habit event is an ordinary Google Calendar event
//! tagged `ttKind=habit`, so Google expands it and every other client sees a
//! normal recurring event. Any number of those can point at one habit through a
//! `ttHabit` property, and completing an occurrence records a repetition.
//!
//! Habit events from before this split have no such property. The first time
//! one is seen it is given a habit of the same name, along with its completion
//! history, without writing anything to Google.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, Timelike, Weekday};
use iced::{Color, Task};

use crate::app::{CalendarMessage, EventMessage};
use crate::app::{Message, State};
use crate::blocking;
use crate::calendar::{self, api::HabitEvent, api::HabitListing, colors, deadline};
use crate::recurrence::Rule;

pub use crate::recurrence::{EndKind, MonthlyMode, Recurrence, RepeatMode, WEEKDAYS};
pub use timeparagon_core::habit_edit::{
	DayDialog, Form, FormAction, FrequencyKind, Log, LogRow, SectionEdit, new_id,
};
pub use timeparagon_core::habits::{
	Bucket, Computed, DayEntry, Frequency, Habit, Icons, Kind, Library, LogEntry, Mark, Measure,
	Outcome, Section, SortKey, Status, Streak,
};

/// The `ttKind` Google extended property that marks a habit event.
pub const KIND: &str = "habit";

/// The week the charts group by, matching the rest of the app.
pub const FIRST_WEEKDAY: Weekday = Weekday::Mon;

/// A habit event, as the master event on the TimeParagon calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
	pub id: String,
	pub calendar_id: String,
	pub title: String,
	pub notes: String,
	pub start: DateTime<Local>,
	pub end: DateTime<Local>,
	pub recurrence: Recurrence,
	pub color_id: Option<String>,
	pub habit_id: Option<String>,
}

impl Series {
	pub fn from_event(event: HabitEvent) -> Self {
		Self {
			id: event.id,
			calendar_id: event.calendar_id,
			title: event.title,
			notes: event.notes,
			start: event.start,
			end: event.end,
			recurrence: Recurrence::from_lines(&event.recurrence),
			color_id: event.color_id,
			habit_id: event.habit_id,
		}
	}

	/// The event's Google color when it has one, otherwise the app's habit
	/// event color.
	pub fn color(&self, settings: &crate::config::CalendarSettings) -> Color {
		self.color_id
			.as_deref()
			.and_then(colors::event)
			.unwrap_or_else(|| settings.habit_color())
	}

	/// A readable description of the recurrence, or "Does not repeat".
	pub fn recurrence_label(&self) -> String {
		if self.recurrence.is_empty() {
			return "Does not repeat".to_owned();
		}
		Draft::from_series(self)
			.rule
			.summary(self.start.date_naive())
	}

	/// The time of day the event runs.
	pub fn time_label(&self) -> String {
		format!(
			"{} – {}",
			self.start.format("%H:%M"),
			self.end.format("%H:%M")
		)
	}

	/// The closest goal to an event's repeat, for a habit made from the event.
	fn frequency(&self) -> Frequency {
		let rule = Rule::from_recurrence(self.start.date_naive(), &self.recurrence);
		let every = rule.interval.trim().parse::<u32>().unwrap_or(1).max(1);
		match rule.repeat {
			RepeatMode::Daily => Frequency::Interval {
				times: 1,
				days: every,
			},
			RepeatMode::Weekly if every == 1 && rule.weekdays.contains(&true) => {
				Frequency::Weekdays {
					days: rule.weekdays,
				}
			}
			RepeatMode::Weekly => Frequency::Interval {
				times: 1,
				days: every * 7,
			},
			RepeatMode::Monthly => Frequency::Interval {
				times: 1,
				days: every * 30,
			},
			RepeatMode::Yearly => Frequency::Interval {
				times: 1,
				days: every * 365,
			},
			_ => Frequency::DAILY,
		}
	}
}

/// A habit event being filled in before it is written to Google.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
	/// `None` when this is a new habit.
	pub id: Option<String>,
	pub calendar_id: Option<String>,
	pub title: String,
	pub notes: String,
	pub date: String,
	pub start: String,
	pub end: String,
	/// The shared repeat controls.
	pub rule: Rule,
	pub color_id: Option<String>,
	pub error: Option<String>,
}

impl Draft {
	pub fn new(day: NaiveDate, color_id: Option<String>) -> Self {
		Self {
			id: None,
			calendar_id: None,
			title: String::new(),
			notes: String::new(),
			date: day.to_string(),
			start: "09:00".to_owned(),
			end: "09:30".to_owned(),
			rule: Rule::weekly(day),
			color_id,
			error: None,
		}
	}

	/// A draft seeded from the calendar's drag-to-create dialog.
	pub fn for_calendar(
		start: DateTime<Local>,
		end: DateTime<Local>,
		title: String,
		notes: String,
		color_id: Option<String>,
	) -> Self {
		let mut draft = Self::new(start.date_naive(), color_id);
		draft.title = title;
		draft.notes = notes;
		draft.start = start.format("%H:%M").to_string();
		draft.end = end.format("%H:%M").to_string();
		draft
	}

	/// The same, carrying the repeat schedule already chosen in the new-event
	/// dialog. A habit always repeats, so an empty schedule becomes weekly on
	/// the start day.
	pub fn for_calendar_with_rule(
		start: DateTime<Local>,
		end: DateTime<Local>,
		title: String,
		notes: String,
		color_id: Option<String>,
		rule: Rule,
	) -> Self {
		let mut draft = Self::for_calendar(start, end, title, notes, color_id);
		draft.rule = rule;
		if draft.rule.repeat == RepeatMode::Never {
			draft.rule = Rule::weekly(start.date_naive());
		}
		draft
	}

	pub fn from_series(habit: &Series) -> Self {
		let mut draft = Self::new(habit.start.date_naive(), habit.color_id.clone());
		draft.id = Some(habit.id.clone());
		draft.calendar_id = Some(habit.calendar_id.clone());
		draft.title = habit.title.clone();
		draft.notes = habit.notes.clone();
		draft.start = habit.start.format("%H:%M").to_string();
		draft.end = habit.end.format("%H:%M").to_string();
		draft.rule = Rule::from_recurrence(habit.start.date_naive(), &habit.recurrence);
		draft
	}

	/// The anchor day for the repeat controls.
	pub fn anchor_date(&self) -> Option<NaiveDate> {
		deadline::parse_date(self.date.trim()).ok()
	}

	/// Reads an imported recurrence into the structured fields. Returns false
	/// for anything the editor cannot show without RFC text, which then stays
	/// untouched as a custom schedule.
	pub fn apply_recurrence(&mut self, recurrence: &Recurrence) -> bool {
		let Some(anchor) = self.anchor_date() else {
			return false;
		};
		self.rule.apply(anchor, recurrence)
	}

	/// Parses the fields into the values the Calendar API write needs.
	pub fn parsed(&self) -> Result<ParsedHabit, String> {
		let title = self.title.trim();
		if title.is_empty() {
			return Err("Give the habit a name".to_owned());
		}
		let date = deadline::parse_date(self.date.trim())?;
		let start = deadline::parse_time(self.start.trim())?;
		let end = deadline::parse_time(self.end.trim())?;
		if end <= start {
			return Err("The end time has to be after the start".to_owned());
		}
		let recurrence = self.recurrence_lines_at(start)?;
		Ok(ParsedHabit {
			title: title.to_owned(),
			date,
			start,
			end,
			recurrence,
		})
	}

	/// Builds the recurrence array the editor describes. The clock time is
	/// used for skipped and extra dates; without a valid one it falls back to
	/// the usual 09:00 default so validation can still report other problems.
	pub fn recurrence_lines(&self) -> Result<Vec<String>, String> {
		let start = deadline::parse_time(self.start.trim())
			.unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
		self.recurrence_lines_at(start)
	}

	fn recurrence_lines_at(&self, start: NaiveTime) -> Result<Vec<String>, String> {
		let date = deadline::parse_date(self.date.trim())?;
		self.rule.lines_at(date, start)
	}

	/// A short, human description used by the cards.
	pub fn repeat_summary(&self) -> String {
		let anchor = self
			.anchor_date()
			.unwrap_or_else(|| crate::clock::now().date_naive());
		self.rule.summary(anchor)
	}
}

/// A draft's fields after parsing, before the Calendar write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHabit {
	pub title: String,
	pub date: NaiveDate,
	pub start: NaiveTime,
	pub end: NaiveTime,
	pub recurrence: Vec<String>,
}

/// A habit's row being pressed, which is a click until the pointer reaches
/// another row, and a move in the manual order from then on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drag {
	pub id: String,
	pub moved: bool,
}

/// The input in the day dialog.
pub const DAY_FIELD: &str = "habit-day-value";

/// The input that names a section.
pub const SECTION_FIELD: &str = "habit-section-name";

/// A drop-down menu that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
	/// The list's sort and filter menu.
	Filter,
	/// The list's overflow menu.
	List,
	/// The open habit's overflow menu.
	Habit,
}

/// What the Habits tab is holding.
#[derive(Debug, Default)]
pub struct Store {
	/// Everything saved to disk.
	pub library: Library,
	/// Each habit's record worked out for display, as of `today`.
	computed: HashMap<String, Computed>,
	today: Option<NaiveDate>,
	/// The file `library` is kept in. Unset in tests, and when the file exists
	/// but cannot be read, so that it is not overwritten.
	path: Option<PathBuf>,
	/// The habit events on the calendar.
	pub series: Vec<Series>,
	/// Whether the events have been fetched at least once.
	pub loaded: bool,
	/// The habit open in the extended view.
	pub selected: Option<String>,
	/// The add and edit page, shown in place of the extended view.
	pub form: Option<Form>,
	pub menu: Option<Menu>,
	pub day_dialog: Option<DayDialog>,
	pub section_edit: Option<SectionEdit>,
	pub dragging: Option<Drag>,
	/// The row last pressed. It outlives `dragging`, because the global release
	/// that ends a drag can arrive before the row's own release does.
	pressed: Option<String>,
	/// The habit whose events the month view is filtered to.
	pub calendar_habit: Option<String>,
	pub completing: HashSet<String>,
	/// A Calendar write is in flight.
	pub busy: bool,
	/// The habits and calendar events whose Calendar write is in flight, so
	/// they can be shown as not yet final.
	pub writing: HashSet<String>,
	pub error: Option<String>,
	/// A deletion waiting for confirmation, by habit id.
	pub confirm_delete: Option<String>,
	/// Whether confirming also deletes the habit's calendar events.
	pub delete_events: bool,
	/// Bumped by every local change that is saved, so a reload read before
	/// the change can tell it is out of date.
	revision: u64,
	/// Saves started and not yet finished.
	saves_in_flight: u32,
}

impl Store {
	pub fn reload_shared(&mut self) {
		let Some(path) = &self.path else {
			return;
		};
		if let Ok(Some(library)) = crate::task_db::read_document(path, crate::task_db::keys::HABITS)
		{
			self.adopt_shared(library);
		}
	}

	/// Takes habits read from the database, when they differ from these.
	/// The caller makes sure the read is not older than a local change.
	pub fn adopt_shared(&mut self, library: Library) {
		if library != self.library {
			self.library = library;
			self.recompute();
		}
	}

	/// Where the habits are saved, when they are.
	pub fn path(&self) -> Option<&std::path::Path> {
		self.path.as_deref()
	}

	/// Changes with each local change that is saved.
	pub fn revision(&self) -> u64 {
		self.revision
	}

	/// Whether a save is still on its way to the database.
	pub fn saving(&self) -> bool {
		self.saves_in_flight > 0
	}

	/// Stands in for a local change being saved.
	#[cfg(test)]
	pub(crate) fn begin_save_for_test(&mut self) {
		self.revision += 1;
		self.saves_in_flight += 1;
	}

	/// Loads the saved habits. Saved habits that cannot be read are left alone
	/// rather than replaced by an empty list on the next save.
	pub fn keep_in(&mut self, path: Option<PathBuf>) {
		let Some(path) = path else { return };
		match crate::task_db::read_document(&path, crate::task_db::keys::HABITS) {
			Ok(library) => {
				self.library = library.unwrap_or_default();
				self.path = Some(path);
			}
			Err(error) => {
				log::warn!("the saved habits could not be read: {error}");
				self.error = Some(
					"The saved habits could not be read, so changes to habits will not be saved"
						.to_owned(),
				);
			}
		}
		self.recompute();
	}

	/// Whether changes reach the disk.
	pub fn persists(&self) -> bool {
		self.path.is_some()
	}

	/// Today, as far as the computed records go.
	pub fn today(&self) -> NaiveDate {
		self.today
			.unwrap_or_else(|| crate::clock::now().date_naive())
	}

	/// Works every record out again.
	pub fn recompute(&mut self) {
		let today = crate::clock::now().date_naive();
		self.today = Some(today);
		self.computed = self
			.library
			.habits
			.iter()
			.map(|habit| (habit.id.clone(), habit.compute(today)))
			.collect();
	}

	fn recompute_one(&mut self, id: &str) {
		let today = self.today();
		if let Some(habit) = self.library.habit(id) {
			self.computed.insert(id.to_owned(), habit.compute(today));
		}
	}

	/// Scores change at midnight, whether or not anything was clicked.
	pub fn roll_over(&mut self) {
		if self.today != Some(crate::clock::now().date_naive()) {
			self.recompute();
		}
	}

	pub fn computed(&self, id: &str) -> Option<&Computed> {
		self.computed.get(id)
	}

	/// The habits to list, in the chosen order.
	pub fn visible(&self) -> Vec<&Habit> {
		self.library
			.visible(&|id| self.computed.get(id), self.today())
	}

	pub fn selected_habit(&self) -> Option<&Habit> {
		self.library.habit(self.selected.as_deref()?)
	}

	/// The habit a series counts towards.
	fn habit_of_series<'a>(&'a self, series: &'a Series) -> Option<&'a str> {
		series
			.habit_id
			.as_deref()
			.filter(|id| self.library.habit(id).is_some())
	}

	/// The habit an occurrence on the calendar counts towards. Its series has
	/// the final say, because a completed occurrence keeps its own copy of the
	/// properties from when it was completed.
	pub fn habit_of<'a>(&'a self, event: &'a calendar::Event) -> Option<&'a str> {
		if event.is_task() {
			return event
				.linked_habit_id()
				.filter(|id| self.library.habit(id).is_some());
		}
		let series_id = event
			.details
			.recurring_event_id
			.as_deref()
			.unwrap_or(&event.id);
		if let Some(series) = self.series.iter().find(|series| series.id == series_id) {
			return self.habit_of_series(series);
		}
		event
			.details
			.habit_id
			.as_deref()
			.filter(|id| self.library.habit(id).is_some())
	}

	/// The events connected to a habit.
	pub fn events_of(&self, habit_id: &str) -> Vec<&Series> {
		self.series
			.iter()
			.filter(|series| self.habit_of_series(series) == Some(habit_id))
			.collect()
	}

	/// Events not connected to any habit.
	pub fn unlinked_events(&self) -> Vec<&Series> {
		self.series
			.iter()
			.filter(|series| self.habit_of_series(series).is_none())
			.collect()
	}

	/// The name of the habit the month view is filtered to.
	pub fn calendar_habit_name(&self) -> Option<&str> {
		let id = self.calendar_habit.as_deref()?;
		self.library.habit(id).map(|habit| habit.name.as_str())
	}

	pub fn matches_calendar_habit(&self, event: &calendar::Event) -> bool {
		let Some(id) = self.calendar_habit.as_deref() else {
			return true;
		};
		event.is_habit() && self.habit_of(event) == Some(id)
	}

	/// Restore a habit explicitly linked by a calendar series on another device.
	fn adopt(&mut self, _listing: &HabitListing) -> bool {
		let mut changed = false;
		for series in &self.series {
			let Some(id) = &series.habit_id else { continue };
			if self.library.habit(id).is_none() {
				let mut habit = Habit::new(id.clone(), series.title.trim().to_owned());
				habit.frequency = series.frequency();
				self.library.habits.push(habit);
				changed = true;
			}
		}
		changed
	}
}

fn write(path: &std::path::Path, library: &Library) -> std::io::Result<()> {
	crate::task_db::write_document(path, crate::task_db::keys::HABITS, library)
}

#[cfg(test)]
pub(crate) fn write_for_test(state: &State, path: &std::path::Path) {
	write(path, &state.habits.library).unwrap();
}

pub(crate) fn persist(state: &mut State) -> Task<Message> {
	let Some(path) = state.habits.path.clone() else {
		return Task::none();
	};
	state.habits.revision += 1;
	state.habits.saves_in_flight += 1;
	let library = state.habits.library.clone();
	blocking::run(move || write(&path, &library).map_err(|error| error.to_string()))
		.map(|result| Message::Habits(Action::Persisted(result)))
}

// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Action {
	Refresh,
	Loaded(Result<HabitListing, String>),
	Persisted(Result<(), String>),

	// The list.
	/// A click on a day anywhere, or the Record button on a habit event: opens
	/// the dialog with everything the day can record.
	OpenDay(String, NaiveDate),
	DayOutcome(Outcome),
	/// One more time, or one fewer.
	StepTimes(bool),
	StepResisted(bool),
	/// Logs an entry at the current time, in the day dialog.
	LogAdd(Log),
	/// Removes an entry from one of the day dialog's logs, by position.
	LogRemove(Log, usize),
	/// Edits the time of an entry in the day dialog.
	LogTime(Log, usize, String),
	/// Edits the amount of an entry in the day dialog.
	LogAmount(Log, usize, String),
	/// Edits the comment on an entry in the day dialog.
	LogNote(Log, usize, String),
	DayValue(String),
	/// Records what the dialog holds.
	SaveDay,
	/// Closes the dialog without recording anything.
	CloseDay,
	Menu(Menu),
	CloseMenu,
	Sort(SortKey),
	HideArchived(bool),
	HideCompleted(bool),
	RowPressed(String),
	RowEntered(String),
	RowReleased(String),
	/// The button was let go somewhere that is not a row.
	DragEnded,

	// Sections of the list.
	NewSection,
	EditSection(String),
	SectionName(String),
	SaveSection,
	CancelSection,
	DeleteSection(String),
	/// Moves a section up the list, or down it.
	ShiftSection(String, bool),
	ToggleSection(String),
	/// The pointer reached a section's heading.
	SectionEntered(String),

	// The extended view.
	Open(String),
	Close,
	ScoreBucket(Bucket),
	HistoryBucket(Bucket),

	// The add and edit page.
	New,
	EditHabit(String),
	CancelEdit,
	Form(FormAction),
	SaveForm,

	// Looking after a habit.
	ToggleArchived(String),
	DeleteRequested(String),
	DeleteEventsToggled(bool),
	DeleteCancelled,
	DeleteConfirmed(String),
	Deleted(String, Result<(), String>),
	/// Exports one habit, or all of them.
	Export(Option<String>),
	ExportFinished(Result<bool, String>),

	// The calendar events connected to a habit.
	AddEvent(String),
	EditEvent(String),
	LinkEvent(String, String),
	UnlinkEvent(String),
	Linked(String, Option<String>, Result<(), String>),
	/// The habit picked for the event open in the shared editor.
	EventHabitChosen(Option<String>),
	ShowOnCalendar(String),
	CloseCalendar,
	ToggleCompleted(String),
	CompletionSaved(String, Result<calendar::Event, String>),
	/// An occurrence brought in line with a click on the habit list.
	OccurrenceSynced(String, Result<calendar::Event, String>),
	Saved(Result<HabitEvent, String>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::Refresh => refresh(state),
		Action::Loaded(result) => {
			state.habits.busy = false;
			state.habits.loaded = true;
			match result {
				Ok(listing) => {
					state.habits.series = listing
						.series
						.iter()
						.cloned()
						.map(Series::from_event)
						.collect();
					sort_series(&mut state.habits.series);
					state.habits.error = None;
					if state.habits.adopt(&listing) {
						state.habits.recompute();
						return persist(state);
					}
				}
				Err(error) => state.habits.error = Some(error),
			}
			Task::none()
		}
		Action::Persisted(result) => {
			state.habits.saves_in_flight = state.habits.saves_in_flight.saturating_sub(1);
			if let Err(error) = result {
				log::warn!("could not save habits: {error}");
				state.habits.error = Some(format!("Could not save habits: {error}"));
			}
			Task::none()
		}

		Action::OpenDay(id, day) => open_day(state, id, day),
		Action::DayOutcome(outcome) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.choose(outcome);
			}
			Task::none()
		}
		Action::StepTimes(up) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.step_times(up);
			}
			Task::none()
		}
		Action::StepResisted(up) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.step_resisted(up);
			}
			Task::none()
		}
		Action::LogAdd(log) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.add(log);
			}
			Task::none()
		}
		Action::LogRemove(log, index) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.remove(log, index);
			}
			Task::none()
		}
		Action::LogTime(log, index, time) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.edit(log, index, |row| row.time = time);
			}
			Task::none()
		}
		Action::LogAmount(log, index, amount) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.edit(log, index, |row| row.amount = amount);
			}
			Task::none()
		}
		Action::LogNote(log, index, note) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.edit(log, index, |row| row.note = note);
			}
			Task::none()
		}
		Action::DayValue(value) => {
			if let Some(dialog) = &mut state.habits.day_dialog {
				dialog.set_value(value);
			}
			Task::none()
		}
		Action::SaveDay => save_day(state),
		Action::CloseDay => {
			state.habits.day_dialog = None;
			Task::none()
		}
		Action::Menu(menu) => {
			state.habits.menu = (state.habits.menu != Some(menu)).then_some(menu);
			Task::none()
		}
		Action::CloseMenu => {
			state.habits.menu = None;
			Task::none()
		}
		Action::Sort(key) => {
			state.habits.library.choose_sort(key);
			state.habits.menu = None;
			persist(state)
		}
		Action::HideArchived(hide) => {
			state.habits.library.hide_archived = hide;
			persist(state)
		}
		Action::HideCompleted(hide) => {
			state.habits.library.hide_completed = hide;
			persist(state)
		}
		Action::RowPressed(id) => {
			state.habits.pressed = Some(id.clone());
			state.habits.dragging = Some(Drag { id, moved: false });
			Task::none()
		}
		Action::RowEntered(over) => {
			if state.habits.library.sort != SortKey::Manual {
				return Task::none();
			}
			let Some(drag) = &mut state.habits.dragging else {
				return Task::none();
			};
			if drag.id != over {
				drag.moved = true;
				let id = drag.id.clone();
				state.habits.library.move_habit(&id, &over);
			}
			Task::none()
		}
		Action::RowReleased(over) => {
			let moved = state.habits.dragging.take().is_some_and(|drag| drag.moved);
			let pressed = state.habits.pressed.take();
			if moved {
				persist(state)
			} else if pressed.as_deref() == Some(over.as_str()) {
				open(state, over)
			} else {
				Task::none()
			}
		}
		Action::DragEnded => match state.habits.dragging.take() {
			Some(drag) if drag.moved => {
				state.habits.pressed = None;
				persist(state)
			}
			_ => Task::none(),
		},

		Action::NewSection => {
			state.habits.menu = None;
			state.habits.section_edit = Some(SectionEdit {
				id: None,
				name: String::new(),
			});
			iced::widget::operation::focus(iced::widget::Id::new(SECTION_FIELD))
		}
		Action::EditSection(id) => {
			let Some(section) = state.habits.library.section_mut(&id) else {
				return Task::none();
			};
			let name = section.name.clone();
			state.habits.section_edit = Some(SectionEdit { id: Some(id), name });
			iced::widget::operation::focus(iced::widget::Id::new(SECTION_FIELD))
		}
		Action::SectionName(name) => {
			if let Some(edit) = &mut state.habits.section_edit {
				edit.name = name;
			}
			Task::none()
		}
		Action::SaveSection => save_section(state),
		Action::CancelSection => {
			state.habits.section_edit = None;
			Task::none()
		}
		Action::DeleteSection(id) => {
			state.habits.section_edit = None;
			state.habits.library.remove_section(&id);
			if let Some(form) = &mut state.habits.form
				&& form.section.as_deref() == Some(id.as_str())
			{
				form.section = None;
			}
			persist(state)
		}
		Action::ShiftSection(id, up) => {
			state.habits.library.shift_section(&id, up);
			persist(state)
		}
		Action::ToggleSection(id) => {
			if let Some(section) = state.habits.library.section_mut(&id) {
				section.collapsed = !section.collapsed;
			}
			persist(state)
		}
		Action::SectionEntered(section) => {
			if state.habits.library.sort != SortKey::Manual {
				return Task::none();
			}
			if let Some(drag) = &mut state.habits.dragging {
				drag.moved = true;
				let id = drag.id.clone();
				state.habits.library.move_to_section(&id, &section);
			}
			Task::none()
		}

		Action::Open(id) => open(state, id),
		Action::Close => {
			state.habits.selected = None;
			state.habits.form = None;
			state.habits.menu = None;
			Task::none()
		}
		Action::ScoreBucket(bucket) => {
			state.habits.library.score_bucket = bucket;
			persist(state)
		}
		Action::HistoryBucket(bucket) => {
			state.habits.library.history_bucket = bucket;
			persist(state)
		}

		Action::New => {
			state.habits.menu = None;
			state.habits.confirm_delete = None;
			state.habits.selected = None;
			state.habits.form = Some(Form::new());
			iced::widget::operation::focus(iced::widget::Id::new(NAME_FIELD))
		}
		Action::EditHabit(id) => {
			state.habits.menu = None;
			let Some(habit) = state.habits.library.habit(&id) else {
				return Task::none();
			};
			state.habits.form = Some(Form::from_habit(habit));
			Task::none()
		}
		Action::CancelEdit => {
			state.habits.form = None;
			Task::none()
		}
		Action::Form(action) => {
			if let Some(form) = &mut state.habits.form {
				form.apply(action);
			}
			Task::none()
		}
		Action::SaveForm => save_form(state),

		Action::ToggleArchived(id) => {
			state.habits.menu = None;
			if let Some(habit) = state.habits.library.habit_mut(&id) {
				habit.archived = !habit.archived;
			}
			persist(state)
		}
		Action::DeleteRequested(id) => {
			state.habits.menu = None;
			state.habits.delete_events = false;
			state.habits.confirm_delete = Some(id);
			Task::none()
		}
		Action::DeleteEventsToggled(delete) => {
			state.habits.delete_events = delete;
			Task::none()
		}
		Action::DeleteCancelled => {
			state.habits.confirm_delete = None;
			Task::none()
		}
		Action::DeleteConfirmed(id) => delete(state, id),
		Action::Deleted(id, result) => {
			state.habits.busy = false;
			state.habits.writing.clear();
			state.habits.confirm_delete = None;
			if let Err(error) = result {
				state.habits.error = Some(error);
				return refresh(state);
			}
			let events: Vec<String> = state
				.habits
				.events_of(&id)
				.iter()
				.map(|series| series.id.clone())
				.collect();
			let sync = !events.is_empty();
			for series in &mut state.habits.series {
				if series.habit_id.as_deref() == Some(id.as_str()) {
					series.habit_id = None;
				}
			}
			state.habits.library.habits.retain(|habit| habit.id != id);
			state.habits.computed.remove(&id);
			if state.habits.selected.as_deref() == Some(id.as_str()) {
				state.habits.selected = None;
			}
			state.habits.error = None;
			let saved = persist(state);
			if sync {
				Task::batch([saved, refresh_after_event_change()])
			} else {
				saved
			}
		}
		Action::Export(id) => {
			state.habits.menu = None;
			export(state, id)
		}
		Action::ExportFinished(result) => {
			if let Err(error) = result {
				state.habits.error = Some(format!("Could not export: {error}"));
			}
			Task::none()
		}

		Action::AddEvent(habit_id) => {
			let Some(habit) = state.habits.library.habit(&habit_id) else {
				return Task::none();
			};
			let day = crate::clock::now().date_naive();
			let color_id = None;
			let mut draft = Draft::new(day, color_id);
			draft.title = habit.name.clone();
			let task = open_new(state, draft);
			if let Some(event) = &mut state.calendar.detail_event {
				event.details.habit_id = Some(habit_id);
			}
			task
		}
		Action::EditEvent(id) => edit_event(state, id),
		Action::LinkEvent(series_id, habit_id) => link(state, series_id, Some(habit_id)),
		Action::UnlinkEvent(series_id) => link(state, series_id, None),
		Action::Linked(series_id, habit_id, result) => {
			state.habits.busy = false;
			state.habits.writing.clear();
			if let Err(error) = result {
				state.habits.error = Some(error);
				return Task::none();
			}
			state.habits.error = None;
			if let Some(series) = state
				.habits
				.series
				.iter_mut()
				.find(|series| series.id == series_id)
			{
				series.habit_id = habit_id.clone();
			}
			refresh_after_event_change()
		}
		Action::EventHabitChosen(habit_id) => event_habit_chosen(state, habit_id),
		Action::ShowOnCalendar(id) => {
			// The month takes the window, so the page gives way until it closes.
			state.habits.selected = None;
			state.calendar.open_event = None;
			state.calendar.detail_event = None;
			state.calendar.edit_draft = None;
			let task = crate::app::update(
				state,
				Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
			);
			// Set afterwards: switching to the calendar screen clears the filter.
			state.habits.calendar_habit = Some(id);
			task
		}
		Action::CloseCalendar => {
			state.habits.selected = state.habits.calendar_habit.take();
			crate::app::update(
				state,
				Message::Calendar(CalendarMessage::CalendarModeSelected(false)),
			)
		}
		Action::ToggleCompleted(id) => {
			if state.habits.completing.contains(&id) {
				return Task::none();
			}
			let Some(event) = state.calendar.event(&id).cloned() else {
				return Task::none();
			};
			if !event.details.habit {
				return Task::none();
			}
			let completed = !event.completed;
			write_occurrence(state, event, completed, Action::CompletionSaved)
		}
		Action::CompletionSaved(id, result) => {
			state.habits.completing.remove(&id);
			match result {
				Ok(event) => {
					state.calendar.replace_habit_occurrence(&event);
					return record_occurrence(state, &event);
				}
				Err(error) => state.habits.error = Some(error),
			}
			Task::none()
		}
		Action::OccurrenceSynced(id, result) => {
			state.habits.completing.remove(&id);
			match result {
				Ok(event) => state.calendar.replace_habit_occurrence(&event),
				Err(error) => state.habits.error = Some(error),
			}
			Task::none()
		}
		Action::Saved(result) => saved_event(state, result),
	}
}

/// The text input that holds a habit's name on the form.
pub const NAME_FIELD: &str = "habit-name";

fn open(state: &mut State, id: String) -> Task<Message> {
	state.habits.selected = Some(id);
	state.habits.form = None;
	state.habits.menu = None;
	state.calendar.open_event = None;
	state.calendar.detail_event = None;
	state.calendar.edit_draft = None;
	Task::none()
}

fn open_day(state: &mut State, id: String, day: NaiveDate) -> Task<Message> {
	let Some(habit) = state.habits.library.habit(&id) else {
		return Task::none();
	};
	let format = state.config.appearance.time_format;
	state.habits.day_dialog = Some(DayDialog::new(habit, day, format));
	if habit.measure.is_some() {
		iced::widget::operation::focus(iced::widget::Id::new(DAY_FIELD))
	} else {
		Task::none()
	}
}

/// Records what the dialog holds, keeping calendar occurrences in step. A value
/// that cannot be read keeps the dialog open to say so.
fn save_day(state: &mut State) -> Task<Message> {
	let Some(dialog) = state.habits.day_dialog.take() else {
		return Task::none();
	};
	if let Err(error) = dialog.record(&mut state.habits.library) {
		state.habits.day_dialog = Some(DayDialog {
			error: Some(error),
			..dialog
		});
		return Task::none();
	}
	let (id, day) = (dialog.habit_id, dialog.day);
	if state.habits.library.habit(&id).is_none() {
		return Task::none();
	}
	state.habits.recompute_one(&id);
	Task::batch([persist(state), sync_occurrences(state, &id, day)])
}

/// Names a section. A blank name leaves things as they were.
fn save_section(state: &mut State) -> Task<Message> {
	let Some(edit) = state.habits.section_edit.take() else {
		return Task::none();
	};
	if edit.save(&mut state.habits.library) {
		persist(state)
	} else {
		Task::none()
	}
}

/// Completes or reopens the occurrences loaded for `day` to match the habit, so
/// a click on the list also shows on the calendar. Local data stays the record:
/// days that are not loaded, or a failed write, leave only the habit changed.
fn sync_occurrences(state: &mut State, id: &str, day: NaiveDate) -> Task<Message> {
	let Some(habit) = state.habits.library.habit(id) else {
		return Task::none();
	};
	// Only a "Yes" habit is done by completing an event.
	if habit.kind != Kind::Yes {
		return Task::none();
	}
	let times = habit.times_on(day) as usize;
	let wanted = match state.habits.computed(id).map(|c| c.status(day)) {
		Some(Status::Yes | Status::Partial(_)) => times,
		_ => 0,
	};
	let occurrences: Vec<calendar::Event> = state
		.calendar
		.events_on(day)
		.iter()
		.filter(|event| {
			event.details.habit
				&& state.habits.habit_of(event) == Some(id)
				&& !state.habits.completing.contains(&event.id)
		})
		.cloned()
		.collect();
	let done = occurrences.iter().filter(|event| event.completed).count();
	let mut writes = Vec::new();
	if done < wanted {
		for event in occurrences
			.iter()
			.filter(|event| !event.completed)
			.take(wanted - done)
		{
			writes.push(write_occurrence(
				state,
				event.clone(),
				true,
				Action::OccurrenceSynced,
			));
		}
	} else {
		for event in occurrences
			.iter()
			.rev()
			.filter(|event| event.completed)
			.take(done - wanted)
		{
			writes.push(write_occurrence(
				state,
				event.clone(),
				false,
				Action::OccurrenceSynced,
			));
		}
	}
	Task::batch(writes)
}

fn write_occurrence(
	state: &mut State,
	mut event: calendar::Event,
	completed: bool,
	done: fn(String, Result<calendar::Event, String>) -> Action,
) -> Task<Message> {
	let id = event.id.clone();
	event.completed = completed;
	event.details.habit_completed_at = completed.then(crate::clock::now);
	state.habits.completing.insert(id.clone());
	state.habits.error = None;
	blocking::run(move || {
		calendar::set_habit_completed(&event)?;
		Ok(event)
	})
	.map(move |result| Message::Habits(done(id.clone(), result)))
}

/// Completing an occurrence on the calendar counts as doing the habit that day.
pub(crate) fn record_occurrence(state: &mut State, event: &calendar::Event) -> Task<Message> {
	let Some(id) = state.habits.habit_of(event).map(str::to_owned) else {
		return Task::none();
	};
	let day = event.start.date_naive();
	let Some(habit) = state.habits.library.habit_mut(&id) else {
		return Task::none();
	};
	if habit.kind != Kind::Yes {
		return Task::none();
	}
	let completed = if event.is_task() {
		crate::timeblocking::task_status(event) == crate::timeblocking::TaskStatus::Completed
	} else {
		event.completed
	};
	if completed {
		habit.add_rep(day);
	} else {
		habit.remove_rep(day);
	}
	state.habits.recompute_one(&id);
	persist(state)
}

fn save_form(state: &mut State) -> Task<Message> {
	let Some(form) = state.habits.form.clone() else {
		return Task::none();
	};
	let today = state.habits.today();
	let id = match form.save(&mut state.habits.library, today) {
		Ok(id) => id,
		Err(error) => {
			if let Some(form) = &mut state.habits.form {
				form.error = Some(error);
			}
			return Task::none();
		}
	};
	state.habits.recompute_one(&id);
	state.habits.form = None;
	state.habits.selected = Some(id);
	persist(state)
}

fn delete(state: &mut State, id: String) -> Task<Message> {
	let events: Vec<(String, String)> = state
		.habits
		.events_of(&id)
		.iter()
		.map(|series| (series.calendar_id.clone(), series.id.clone()))
		.collect();
	if events.is_empty() {
		return Task::done(Message::Habits(Action::Deleted(id, Ok(()))));
	}
	let delete_events = state.habits.delete_events;
	let client = calendar::GoogleClient::from_settings(&state.config.calendar);
	state.habits.busy = true;
	// The habit, and the events going with it, show as being deleted until
	// Google answers, rather than behind the dialog.
	state.habits.confirm_delete = None;
	state.habits.writing.insert(id.clone());
	if delete_events {
		state
			.habits
			.writing
			.extend(events.iter().map(|(_, series)| series.clone()));
	}
	state.habits.error = None;
	let result_id = id.clone();
	blocking::run(move || {
		for (calendar_id, event_id) in &events {
			if delete_events {
				calendar::delete_habit(calendar_id, event_id)?;
			} else {
				// A kept event must not bring the habit back on the next load.
				calendar::link_habit_event(&client, calendar_id, event_id, None)?;
			}
		}
		Ok(())
	})
	.map(move |result| Message::Habits(Action::Deleted(result_id.clone(), result)))
}

fn export(state: &State, id: Option<String>) -> Task<Message> {
	let library = &state.habits.library;
	let habits: Vec<Habit> = match &id {
		Some(id) => library.habit(id).cloned().into_iter().collect(),
		None => library.habits.clone(),
	};
	let sections = library.sections.clone();
	let today = state.habits.today();
	let name = match &habits[..] {
		[habit] if id.is_some() => format!("{} {today}.zip", habit.name.replace('/', "-")),
		_ => format!("TimeParagon Habits {today}.zip"),
	};
	blocking::run(move || {
		let Some(file) = rfd::FileDialog::new()
			.set_title("Export habits")
			.set_file_name(&name)
			.add_filter("ZIP archive", &["zip"])
			.save_file()
		else {
			return Ok(false);
		};
		let habits: Vec<&Habit> = habits.iter().collect();
		let bytes = timeparagon_core::habits::export_archive(&habits, &sections, today);
		std::fs::write(file, bytes)
			.map(|()| true)
			.map_err(|error| error.to_string())
	})
	.map(|result| Message::Habits(Action::ExportFinished(result)))
}

fn link(state: &mut State, series_id: String, habit_id: Option<String>) -> Task<Message> {
	let Some(series) = state
		.habits
		.series
		.iter()
		.find(|series| series.id == series_id)
	else {
		return Task::none();
	};
	let calendar_id = series.calendar_id.clone();
	write_link(state, calendar_id, series_id, habit_id)
}

fn write_link(
	state: &mut State,
	calendar_id: String,
	series_id: String,
	habit_id: Option<String>,
) -> Task<Message> {
	let client = calendar::GoogleClient::from_settings(&state.config.calendar);
	state.habits.busy = true;
	state.habits.writing.insert(series_id.clone());
	state.habits.error = None;
	let event_id = series_id.clone();
	let link = habit_id.clone();
	blocking::run(move || {
		calendar::link_habit_event(&client, &calendar_id, &event_id, link.as_deref())
	})
	.map(move |result| Message::Habits(Action::Linked(series_id.clone(), habit_id.clone(), result)))
}

/// The grids hold occurrences of these events, and the list holds the masters.
fn refresh_after_event_change() -> Task<Message> {
	Task::batch([
		Task::done(Message::Habits(Action::Refresh)),
		Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested)),
	])
}

/// The habit picker in the shared event editor. An event being created keeps
/// the choice until it is saved; one that exists is changed straight away, on
/// its whole series.
fn event_habit_chosen(state: &mut State, habit_id: Option<String>) -> Task<Message> {
	if state.calendar.open_event.as_deref() == Some(NEW_HABIT_ID) {
		if let Some(event) = &mut state.calendar.detail_event {
			event.details.habit_id = habit_id;
		}
		return Task::none();
	}
	let Some(event) = state
		.calendar
		.open_event
		.as_deref()
		.and_then(|id| state.calendar.event(id))
	else {
		return Task::none();
	};
	let series_id = event
		.details
		.recurring_event_id
		.clone()
		.unwrap_or_else(|| event.id.clone());
	let calendar_id = event.calendar_id.clone();
	write_link(state, calendar_id, series_id, habit_id)
}

/// Opens a habit event in the shared editor.
fn edit_event(state: &mut State, id: String) -> Task<Message> {
	let Some(series) = state.habits.series.iter().find(|series| series.id == id) else {
		return Task::none();
	};
	let event = calendar::Event {
		id: series.id.clone(),
		calendar_id: series.calendar_id.clone(),
		title: series.title.clone(),
		start: series.start,
		end: series.end,
		all_day: false,
		color: series.color(&state.config.calendar),
		completed: false,
		details: calendar::Details {
			habit: true,
			habit_id: series.habit_id.clone(),
			description: series.notes.clone(),
			recurrence: series.recurrence.lines().to_vec(),
			// Load the full master, including projects and location,
			// through the same path as a calendar occurrence.
			recurring_event_id: Some(series.id.clone()),
			color_id: series.color_id.clone(),
			..Default::default()
		},
	};
	state.habits.confirm_delete = None;
	state.habits.error = None;
	state.calendar.detail_event = Some(event);
	crate::app::update(state, Message::Event(EventMessage::EventOpened(id)))
}

fn saved_event(state: &mut State, result: Result<HabitEvent, String>) -> Task<Message> {
	state.habits.busy = false;
	state.habits.writing.clear();
	match result {
		Ok(event) => {
			let series = Series::from_event(event);
			state.habits.error = None;
			if state.calendar.open_event.as_deref() == Some(NEW_HABIT_ID) {
				state.calendar.open_event = None;
				state.calendar.edit_draft = None;
				state.calendar.detail_event = None;
			}
			if let Some(existing) = state
				.habits
				.series
				.iter_mut()
				.find(|candidate| candidate.id == series.id)
			{
				*existing = series;
			} else {
				state.habits.series.push(series);
			}
			sort_series(&mut state.habits.series);
			// The day and week grids hold instances of this series.
			Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested))
		}
		Err(error) => {
			if state.calendar.open_event.as_deref() == Some(NEW_HABIT_ID) {
				state.calendar.error = Some(error);
			} else {
				state.habits.error = Some(error);
			}
			Task::none()
		}
	}
}

/// Which habit a new event should count towards when it is saved, creating one
/// from the event's title if none was picked. This is what lets a habit event
/// be made straight from the calendar.
pub fn habit_for_new_event(state: &mut State, title: &str) -> (String, Task<Message>) {
	let chosen = state
		.calendar
		.detail_event
		.as_ref()
		.and_then(|event| event.details.habit_id.clone())
		.filter(|id| state.habits.library.habit(id).is_some());
	if let Some(id) = chosen {
		return (id, Task::none());
	}
	habit_for_title(state, title)
}

/// Find or create a habit by name for a new calendar entry.
pub(crate) fn habit_for_title(state: &mut State, title: &str) -> (String, Task<Message>) {
	if let Some(habit) = state.habits.library.named(title) {
		return (habit.id.clone(), Task::none());
	}
	let id = new_id();
	state
		.habits
		.library
		.habits
		.push(Habit::new(id.clone(), title.trim().to_owned()));
	state.habits.recompute_one(&id);
	(id, persist(state))
}

/// Unsaved habit events never enter the calendar grid or its disk cache.
pub const NEW_HABIT_ID: &str = "timeparagon:new-habit";

pub fn open_new(state: &mut State, draft: Draft) -> Task<Message> {
	let day = draft
		.anchor_date()
		.unwrap_or_else(|| crate::clock::now().date_naive());
	let start =
		deadline::parse_time(&draft.start).unwrap_or(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
	let end =
		deadline::parse_time(&draft.end).unwrap_or(NaiveTime::from_hms_opt(9, 30, 0).unwrap());
	let event = calendar::Event {
		id: NEW_HABIT_ID.into(),
		calendar_id: state
			.config
			.calendar
			.app_calendar
			.clone()
			.unwrap_or_default(),
		title: draft.title,
		start: calendar::at_clock(day, start.hour(), start.minute()),
		end: calendar::at_clock(day, end.hour(), end.minute()),
		all_day: false,
		color: state.config.calendar.habit_color(),
		completed: false,
		details: calendar::Details {
			habit: true,
			description: draft.notes,
			color_id: draft.color_id,
			..Default::default()
		},
	};
	state.habits.error = None;
	state.calendar.error = None;
	state.calendar.detail_event = Some(event);
	let task = crate::app::update(
		state,
		Message::Event(EventMessage::EventOpened(NEW_HABIT_ID.into())),
	);
	if let Some(edit) = state.calendar.edit_draft.as_mut() {
		edit.rule = draft.rule;
		edit.editing_title = true;
	}
	Task::batch([
		task,
		iced::widget::operation::focus(iced::widget::Id::new("event-title")),
	])
}

fn refresh(state: &mut State) -> Task<Message> {
	if state.habits.busy {
		return Task::none();
	}
	// Habits work without Google; only their calendar events need it.
	if !state.calendar.is_signed_in() {
		return Task::none();
	}
	state.habits.busy = true;
	let client = calendar::GoogleClient::from_settings(&state.config.calendar);
	let app_calendar = state.config.calendar.app_calendar.clone();
	blocking::run(move || calendar::sync_habit_listing(&client, app_calendar.as_deref()))
		.map(|result| Message::Habits(Action::Loaded(result)))
}

fn sort_series(series: &mut [Series]) {
	series.sort_by(|left, right| {
		// Trimmed like `Library::named`, so events sharing a habit sit together.
		left.title
			.trim()
			.to_lowercase()
			.cmp(&right.title.trim().to_lowercase())
			.then(left.start.cmp(&right.start))
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::recurrence::weekday_flags;

	fn date(text: &str) -> NaiveDate {
		deadline::parse_date(text).unwrap()
	}

	fn draft() -> Draft {
		let mut draft = Draft::new(date("2026-10-28"), Some("3".to_owned()));
		draft.title = "Stretch".to_owned();
		draft
	}

	#[test]
	fn planned_habit_completion_counts_once_and_ignoring_removes_it() {
		let mut state = State::blank();
		state.habits.library.habits.push(Habit::new("read", "Read"));
		let mut task = calendar::sample_snapshot().events.remove(0);
		task.id = "task:read".into();
		task.calendar_id = "tasks:list".into();
		task.details.task_data = Some(timeparagon_core::task_db::TaskData {
			id: 1,
			metadata: crate::timeblocking::Metadata {
				habit_id: Some("read".into()),
				..Default::default()
			},
		});
		let day = task.start.date_naive();
		state.calendar.add_event(task.clone());
		crate::timeblocking::apply_status(&mut task, crate::timeblocking::TaskStatus::Completed);
		let _ = crate::app::update(
			&mut state,
			Message::Task(crate::app::TaskMessage::TaskCompletionSaved(
				task.id.clone(),
				crate::timeblocking::TaskStatus::Active,
				Ok(task.clone()),
			)),
		);
		assert_eq!(state.habits.library.habit("read").unwrap().times_on(day), 1);
		// A save that did not transition completion cannot double count.
		let _ = crate::app::update(
			&mut state,
			Message::Task(crate::app::TaskMessage::TaskCompletionSaved(
				task.id.clone(),
				crate::timeblocking::TaskStatus::Completed,
				Ok(task.clone()),
			)),
		);
		assert_eq!(state.habits.library.habit("read").unwrap().times_on(day), 1);
		crate::timeblocking::apply_status(&mut task, crate::timeblocking::TaskStatus::Ignored);
		let _ = crate::app::update(
			&mut state,
			Message::Task(crate::app::TaskMessage::TaskCompletionSaved(
				task.id.clone(),
				crate::timeblocking::TaskStatus::Completed,
				Ok(task),
			)),
		);
		assert_eq!(state.habits.library.habit("read").unwrap().times_on(day), 0);
	}

	#[test]
	fn common_rules_round_trip_through_the_structured_editor() {
		for (rule, expected) in [
			(
				"RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE",
				"Every 2 weeks on Mon, Wed",
			),
			("RRULE:FREQ=DAILY", "Daily"),
			("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "Every weekday"),
			("RRULE:FREQ=MONTHLY;BYMONTHDAY=28", "Monthly on day 28"),
			(
				"RRULE:FREQ=MONTHLY;BYDAY=4WE",
				"Monthly on the fourth Wednesday",
			),
			(
				"RRULE:FREQ=MONTHLY;BYMONTHDAY=-1",
				"Monthly on the last day",
			),
			("RRULE:FREQ=YEARLY", "Yearly on October 28"),
		] {
			let habit = Series {
				id: "1".into(),
				calendar_id: "timeparagon".into(),
				title: "Stretch".into(),
				notes: String::new(),
				start: calendar::at_clock(date("2026-10-28"), 9, 0),
				end: calendar::at_clock(date("2026-10-28"), 9, 30),
				recurrence: Recurrence::from_lines(&[rule.to_owned()]),
				color_id: None,
				habit_id: None,
			};
			let mut draft = Draft::from_series(&habit);
			assert_eq!(draft.repeat_summary(), expected, "{rule}");
			let lines = draft.recurrence_lines().unwrap();
			assert!(
				lines.iter().all(|line| line.starts_with("RRULE:")),
				"{rule}"
			);
			// Reopening the written rule keeps the same readable summary.
			draft.rule.custom = None;
			let rewritten = Recurrence::from_lines(&lines);
			assert!(draft.apply_recurrence(&rewritten), "{rule}");
			assert_eq!(draft.repeat_summary(), expected, "{rule}");
		}
	}

	#[test]
	fn ends_and_skips_are_written_without_showing_rfc_text() {
		let mut draft = draft();
		draft.rule.repeat = RepeatMode::Weekly;
		draft.rule.weekdays = weekday_flags(&[Weekday::Mon, Weekday::Wed]);
		draft.rule.interval = "2".into();
		draft.rule.ends = EndKind::OnDate;
		draft.rule.until = "2026-12-31".into();
		draft.rule.skips = vec![date("2026-11-11")];
		let lines = draft.recurrence_lines().unwrap();
		let (rule, until) = lines[0].split_once(";UNTIL=").unwrap();
		assert_eq!(rule, "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE");
		// UNTIL is UTC, but the selected end date means the end of the local day.
		// Its UTC representation depends on the machine's time zone.
		let until = chrono::NaiveDateTime::parse_from_str(until, "%Y%m%dT%H%M%SZ")
			.unwrap()
			.and_utc()
			.with_timezone(&Local);
		assert_eq!(
			until.naive_local(),
			date("2026-12-31").and_hms_opt(23, 59, 59).unwrap()
		);
		assert_eq!(lines[1], "EXDATE:20261111T090000");
		assert!(draft.repeat_summary().contains("until December 31"));
		assert!(draft.repeat_summary().contains("1 skipped"));
	}

	#[test]
	fn weekly_needs_at_least_one_day_and_monthly_needs_a_real_day() {
		let mut draft = draft();
		draft.rule.repeat = RepeatMode::Weekly;
		draft.rule.weekdays = [false; 7];
		assert!(draft.parsed().is_err());
		draft.rule.monthly = MonthlyMode::Day;
		draft.rule.monthly_day = "32".into();
		draft.rule.repeat = RepeatMode::Monthly;
		assert!(draft.parsed().is_err());
		draft.rule.monthly_day = "28".into();
		assert!(draft.parsed().is_ok());
	}

	#[test]
	fn an_unsupported_rule_stays_custom_until_the_user_simplifies_it() {
		let recurrence = Recurrence::from_lines(&[
			"RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1".to_owned(),
			"RDATE;VALUE=DATE:20261102".to_owned(),
		]);
		let habit = Series {
			id: "1".into(),
			calendar_id: "timeparagon".into(),
			title: "Stretch".into(),
			notes: String::new(),
			start: calendar::at_clock(date("2026-10-28"), 9, 0),
			end: calendar::at_clock(date("2026-10-28"), 9, 30),
			recurrence: recurrence.clone(),
			color_id: None,
			habit_id: None,
		};
		let mut draft = Draft::from_series(&habit);
		assert_eq!(draft.rule.repeat, RepeatMode::Custom);
		assert_eq!(draft.repeat_summary(), "Custom schedule");
		assert_eq!(draft.recurrence_lines().unwrap(), recurrence.lines());

		draft.rule.repeat = RepeatMode::Weekly;
		draft.rule.custom = None;
		assert!(draft.recurrence_lines().unwrap()[0].starts_with("RRULE:FREQ=WEEKLY"));
	}

	#[test]
	fn lines_that_are_not_recurrence_parts_are_ignored() {
		let recurrence = Recurrence::from_lines(&[
			"RÉSUMÉ".into(),
			"not a rule".into(),
			"RDATE:20261102".into(),
		]);
		assert_eq!(recurrence.lines(), ["RDATE:20261102"]);
	}

	#[test]
	fn imported_rdates_survive_without_being_shown() {
		let lines = vec![
			"RRULE:FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2".to_owned(),
			"RDATE;VALUE=DATE:20261102,20261109".to_owned(),
			"EXDATE;TZID=Europe/London:20261111T090000".to_owned(),
		];
		let recurrence = Recurrence::from_lines(&lines);
		let habit = Series {
			id: "1".into(),
			calendar_id: "timeparagon".into(),
			title: "Stretch".into(),
			notes: String::new(),
			start: calendar::at_clock(date("2026-10-28"), 9, 0),
			end: calendar::at_clock(date("2026-10-28"), 9, 30),
			recurrence,
			color_id: None,
			habit_id: None,
		};
		let draft = Draft::from_series(&habit);
		assert_eq!(draft.rule.repeat, RepeatMode::Weekly);
		assert_eq!(draft.rule.extra, [date("2026-11-02"), date("2026-11-09")]);
		assert_eq!(draft.rule.skips, [date("2026-11-11")]);
		let written = draft.recurrence_lines().unwrap();
		assert!(written.contains(&lines[1]));
		assert!(written.contains(&lines[2]));
	}

	#[test]
	fn a_malformed_schedule_is_reported_before_anything_is_written() {
		let mut draft = draft();
		draft.rule.repeat = RepeatMode::Daily;
		draft.rule.interval = "0".into();
		assert!(draft.parsed().is_err());
		draft.rule.interval = "1".into();
		draft.rule.ends = EndKind::After;
		draft.rule.count = "0".into();
		assert!(draft.parsed().is_err());
		draft.rule.count = "5".into();
		draft.rule.ends = EndKind::OnDate;
		draft.rule.until = "2026-10-01".into();
		assert!(draft.parsed().is_err());
	}

	#[test]
	fn a_draft_needs_a_name_a_real_day_and_an_end_after_the_start() {
		let mut draft = draft();
		draft.title = "  ".into();
		assert!(draft.parsed().is_err());
		draft.title = "Stretch".into();
		draft.date = "2026-02-30".into();
		assert!(draft.parsed().is_err());
		draft.date = "2026-10-28".into();
		draft.end = draft.start.clone();
		assert!(draft.parsed().is_err());
		draft.end = "10:00".into();
		assert_eq!(
			draft.parsed().unwrap().start,
			deadline::parse_time("09:00").unwrap()
		);
	}

	#[test]
	fn the_recurrence_label_is_readable_without_rfc_details() {
		let habit = Series {
			id: "1".into(),
			calendar_id: "timeparagon".into(),
			title: "Stretch".into(),
			notes: String::new(),
			start: calendar::at_clock(date("2026-10-28"), 9, 0),
			end: calendar::at_clock(date("2026-10-28"), 9, 30),
			recurrence: Recurrence::from_lines(&["RRULE:FREQ=DAILY".into()]),
			color_id: None,
			habit_id: None,
		};
		assert_eq!(habit.recurrence_label(), "Daily");
		assert_eq!(habit.time_label(), "09:00 – 09:30");
		let one_off = Series {
			recurrence: Recurrence::default(),
			..habit
		};
		assert_eq!(one_off.recurrence_label(), "Does not repeat");
	}
}
