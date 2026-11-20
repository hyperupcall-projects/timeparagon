//! Picture- and text-to-event extraction and a review queue. Nothing is saved
//! until Add.
use crate::{
	app::{Message, State},
	blocking, calendar, inbox,
};
use iced::{
	Element, Task,
	widget::{Space, button, column, row, text, text_editor},
};
use std::process::{Command, Stdio};

#[derive(Default)]
pub struct Session {
	/// The picture sources, while they are offered.
	pub menu: bool,
	/// The box for typing or pasting text to read events from, while it is open.
	pub text_open: bool,
	pub text: text_editor::Content,
	/// A picture or text is being read: `reading` says which.
	pub busy: bool,
	reading: Origin,
	pub error: Option<String>,
	pub events: Vec<calendar::EditDraft>,
	pub selected: Option<usize>,
	generation: u64,
}
/// What the proposed events were read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Origin {
	#[default]
	Picture,
	Text,
}
impl Origin {
	fn noun(self) -> &'static str {
		match self {
			Origin::Picture => "picture",
			Origin::Text => "text",
		}
	}
}
#[derive(Debug, Clone, Copy)]
pub enum Source {
	Camera,
	Clipboard,
	File,
}
#[derive(Debug, Clone)]
pub enum Action {
	Menu,
	Import(Source),
	/// Open or close the text box.
	TextMenu,
	TextEdit(text_editor::Action),
	/// Read the events out of the text box.
	ImportText,
	Loaded(u64, Result<Option<Vec<Found>>, String>),
	Select(usize),
	Remove(usize),
}
pub use timeparagon_core::ai_input::image::Found;
fn msg(a: Action) -> Message {
	Message::Inbox(inbox::Action::Image(a))
}
fn locked(state: &State) -> bool {
	state.inbox.image.busy
		|| state.inbox.voice.active()
		|| state
			.inbox
			.draft
			.as_ref()
			.is_none_or(|d| d.saving || d.fetching)
}
pub(crate) fn stash(state: &mut State) {
	if let Some(i) = state.inbox.image.selected
		&& state
			.inbox
			.draft
			.as_ref()
			.is_some_and(|d| d.kind == Some(calendar::DraftKind::Event))
		&& let Some(draft) = inbox::creation::entry(state).cloned()
		&& let Some(slot) = state.inbox.image.events.get_mut(i)
	{
		*slot = draft;
	}
}
fn select(state: &mut State, i: usize) {
	stash(state);
	if i >= state.inbox.image.events.len() {
		return;
	}
	let _ = inbox::creation::select(state, calendar::DraftKind::Event);
	// The form may not have been able to take the event on just yet.
	if state.inbox.draft.as_ref().and_then(|d| d.kind) != Some(calendar::DraftKind::Event) {
		return;
	}
	if let Some(draft) = state.inbox.image.events.get(i).cloned() {
		state.calendar.edit_draft = Some(draft);
		state.inbox.image.selected = Some(i);
		state.calendar.error = None;
	}
}
/// Remove the saved proposal and keep the rest open for individual review.
pub fn saved(state: &mut State) -> bool {
	if state
		.inbox
		.draft
		.as_ref()
		.is_none_or(|d| d.kind != Some(calendar::DraftKind::Event))
	{
		return false;
	}
	let Some(i) = state.inbox.image.selected.take() else {
		return false;
	};
	state.inbox.image.events.remove(i);
	if state.inbox.image.events.is_empty() {
		return false;
	}
	state.inbox.draft.as_mut().unwrap().saving = false;
	select(state, i.min(state.inbox.image.events.len() - 1));
	true
}
pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::Loaded(id, result) => {
			if !state.inbox.image.busy || state.inbox.image.generation != id {
				return Task::none();
			}
			state.inbox.image.busy = false;
			let origin = state.inbox.image.reading;
			match result {
				Ok(Some(found)) => {
					let calendar_id = state
						.calendar
						.default_calendar_id(&state.config.calendar)
						.unwrap_or_default();
					let drafts: Result<Vec<_>, _> = found
						.into_iter()
						.map(|f| draft(f, calendar_id.clone(), origin))
						.collect();
					match drafts {
						Ok(drafts) if drafts.is_empty() => {
							state.inbox.image.error = Some(match origin {
								Origin::Picture => "No events found. Try a clearer picture.".into(),
								Origin::Text => "No events found in this text.".into(),
							})
						}
						Ok(drafts) => {
							// The text has become the proposals to review.
							if origin == Origin::Text {
								state.inbox.image.text = text_editor::Content::new();
								state.inbox.image.text_open = false;
							}
							stash(state);
							let first = state.inbox.image.events.len();
							state.inbox.image.events.extend(drafts);
							select(state, first);
						}
						Err(e) => state.inbox.image.error = Some(e),
					}
				}
				Ok(None) => {}
				Err(e) => state.inbox.image.error = Some(e),
			}
		}
		_ if locked(state) => {}
		Action::Menu => {
			state.inbox.image.menu = !state.inbox.image.menu;
			state.inbox.image.text_open = false;
		}
		Action::Import(source) => {
			let id = begin(state, Origin::Picture);
			let settings = state.config.voice.clone();
			return blocking::run(move || import(source, &settings).map_err(|e| e.to_string()))
				.map(move |r| msg(Action::Loaded(id, r)));
		}
		Action::TextMenu => {
			state.inbox.image.text_open = !state.inbox.image.text_open;
			state.inbox.image.menu = false;
			if state.inbox.image.text_open {
				return iced::widget::operation::focus(iced::widget::Id::new(TEXT_FIELD));
			}
		}
		Action::TextEdit(action) => state.inbox.image.text.perform(action),
		Action::ImportText => {
			let text = state.inbox.image.text.text();
			if text.trim().is_empty() {
				return Task::none();
			}
			let id = begin(state, Origin::Text);
			let settings = state.config.voice.clone();
			return blocking::run(move || {
				timeparagon_core::ai_input::text::extract(&text, &settings)
					.map(Some)
					.map_err(|e| e.to_string())
			})
			.map(move |r| msg(Action::Loaded(id, r)));
		}
		Action::Select(i) => select(state, i),
		Action::Remove(i) => {
			if i >= state.inbox.image.events.len() {
				return Task::none();
			}
			stash(state);
			let selected = state.inbox.image.selected;
			state.inbox.image.events.remove(i);
			state.inbox.image.selected = None;
			if state.inbox.image.events.is_empty() {
				inbox::creation::reset_event_entry(state);
			} else {
				let next = selected
					.map(|s| if s > i { s - 1 } else { s })
					.unwrap_or(0)
					.min(state.inbox.image.events.len() - 1);
				select(state, next);
			}
		}
	}
	Task::none()
}
/// Marks a read as started, returning the id its answer must carry.
fn begin(state: &mut State, origin: Origin) -> u64 {
	// Globally unique across closing and reopening a creation session.
	static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
	let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
	let session = &mut state.inbox.image;
	session.menu = false;
	session.error = None;
	session.busy = true;
	session.reading = origin;
	session.generation = id;
	id
}
const TEXT_FIELD: &str = "text-import-field";
/// Sits beside the microphone, and opens the box for text to read events from.
pub fn text_button(state: &State) -> Element<'_, Message> {
	let control = iced::widget::button(iced::widget::canvas(Lines).width(24).height(24))
		.padding(8)
		.style(if state.inbox.image.text_open {
			crate::ui::controls::primary
		} else {
			crate::ui::controls::secondary
		})
		.on_press_maybe((!locked(state)).then_some(msg(Action::TextMenu)));
	iced::widget::tooltip(
		iced::widget::container(control).id("text-import"),
		"Import events from text",
		iced::widget::tooltip::Position::Bottom,
	)
	.into()
}
pub fn input_button(state: &State) -> Element<'_, Message> {
	let control = iced::widget::button(iced::widget::canvas(Picture).width(24).height(24))
		.padding(8)
		.style(crate::ui::controls::secondary)
		.on_press_maybe((!locked(state)).then_some(msg(Action::Menu)));
	iced::widget::tooltip(
		iced::widget::container(control).id("image-import"),
		"Import events from a picture",
		iced::widget::tooltip::Position::Bottom,
	)
	.into()
}
pub fn sources(state: &State) -> Element<'_, Message> {
	let mut body = column![].spacing(4);
	if state.inbox.image.menu {
		body = body
			.push(
				row![
					button("Take picture").on_press(msg(Action::Import(Source::Camera))),
					button("Paste image").on_press(msg(Action::Import(Source::Clipboard))),
				]
				.spacing(4),
			)
			.push(button("Choose image file…").on_press(msg(Action::Import(Source::File))));
	}
	if state.inbox.image.text_open {
		let session = &state.inbox.image;
		let mut editor = crate::ui::controls::text_editor(&session.text)
			.id(iced::widget::Id::new(TEXT_FIELD))
			.placeholder("Paste an email, message or schedule…")
			.height(120);
		if !locked(state) {
			editor = editor.on_action(|a| msg(Action::TextEdit(a)));
		}
		let ready = !locked(state) && !session.text.text().trim().is_empty();
		body = body.push(editor).push(
			row![
				button("Find events").on_press_maybe(ready.then_some(msg(Action::ImportText))),
				button("Close")
					.style(crate::ui::controls::secondary)
					.on_press_maybe((!locked(state)).then_some(msg(Action::TextMenu))),
			]
			.spacing(4),
		);
	}
	if state.inbox.image.busy {
		body = body.push(
			text(match state.inbox.image.reading {
				Origin::Picture => "Reading picture…",
				Origin::Text => "Reading text…",
			})
			.size(12),
		);
	}
	if let Some(e) = &state.inbox.image.error {
		body = body.push(text(e).size(12).style(text::danger));
	}
	body.into()
}
pub fn review(state: &State) -> Element<'_, Message> {
	if state.inbox.image.events.is_empty() {
		return Space::new().into();
	}
	let mut list = column![
		text(format!(
			"{} detected events · review and add individually",
			state.inbox.image.events.len()
		))
		.size(12)
	]
	.spacing(4);
	for (i, cached) in state.inbox.image.events.iter().enumerate() {
		let d = if state.inbox.image.selected == Some(i) {
			inbox::creation::entry(state)
				.filter(|entry| entry.id == inbox::ENTRY_EVENT_ID)
				.unwrap_or(cached)
		} else {
			cached
		};
		list = list.push(
			row![
				button(
					text(format!(
						"{} · {} {}",
						d.title,
						d.date_text,
						if d.all_day { "All day" } else { &d.start_text }
					))
					.size(12)
				)
				.width(iced::Fill)
				.style(crate::ui::controls::nav(
					state.inbox.image.selected == Some(i)
				))
				.on_press_maybe((!locked(state)).then_some(msg(Action::Select(i)))),
				button("×").on_press_maybe((!locked(state)).then_some(msg(Action::Remove(i))))
			]
			.spacing(4),
		);
	}
	iced::widget::container(crate::ui::controls::scrollable(list).height(iced::Length::Shrink))
		.max_height(180)
		.padding([4, 16])
		.into()
}
/// A page of text.
struct Lines;
impl iced::widget::canvas::Program<Message> for Lines {
	type State = ();
	fn draw(
		&self,
		_: &(),
		renderer: &iced::Renderer,
		theme: &iced::Theme,
		bounds: iced::Rectangle,
		_: iced::mouse::Cursor,
	) -> Vec<iced::widget::canvas::Geometry> {
		use iced::Point;
		use iced::widget::canvas::{Frame, Path, Stroke};
		let mut f = Frame::new(renderer, bounds.size());
		let stroke = Stroke::default()
			.with_color(theme.palette().text)
			.with_width(1.6);
		f.stroke(
			&Path::new(|p| {
				p.move_to(Point::new(5., 3.));
				p.line_to(Point::new(15., 3.));
				p.line_to(Point::new(19., 7.));
				p.line_to(Point::new(19., 21.));
				p.line_to(Point::new(5., 21.));
				p.close();
				p.move_to(Point::new(15., 3.));
				p.line_to(Point::new(15., 7.));
				p.line_to(Point::new(19., 7.));
				for (y, end) in [(11., 16.), (14., 16.), (17., 13.)] {
					p.move_to(Point::new(8., y));
					p.line_to(Point::new(end, y));
				}
			}),
			stroke,
		);
		vec![f.into_geometry()]
	}
}
struct Picture;
impl iced::widget::canvas::Program<Message> for Picture {
	type State = ();
	fn draw(
		&self,
		_: &(),
		renderer: &iced::Renderer,
		theme: &iced::Theme,
		bounds: iced::Rectangle,
		_: iced::mouse::Cursor,
	) -> Vec<iced::widget::canvas::Geometry> {
		use iced::widget::canvas::{Frame, Path, Stroke};
		let mut f = Frame::new(renderer, bounds.size());
		let stroke = Stroke::default()
			.with_color(theme.palette().text)
			.with_width(1.6);
		f.stroke(
			&Path::rectangle(iced::Point::new(3., 4.), iced::Size::new(18., 16.)),
			stroke,
		);
		f.stroke(
			&Path::new(|p| {
				p.move_to(iced::Point::new(4., 18.));
				p.line_to(iced::Point::new(10., 11.));
				p.line_to(iced::Point::new(14., 15.));
				p.line_to(iced::Point::new(17., 12.));
				p.line_to(iced::Point::new(20., 16.));
			}),
			stroke,
		);
		f.fill(
			&Path::circle(iced::Point::new(16., 8.), 1.5),
			theme.palette().text,
		);
		vec![f.into_geometry()]
	}
}
fn import(source: Source, settings: &crate::voice::Settings) -> anyhow::Result<Option<Vec<Found>>> {
	let bytes = match source {
		Source::File => {
			let Some(path) = rfd::FileDialog::new()
				.add_filter("Pictures", &["png", "jpg", "jpeg", "webp"])
				.pick_file()
			else {
				return Ok(None);
			};
			anyhow::ensure!(
				std::fs::metadata(&path)?.len() <= 20 * 1024 * 1024,
				"Picture exceeds 20 MB"
			);
			std::fs::read(path)?
		}
		Source::Clipboard => {
			let output = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
				Command::new("wl-paste")
					.args(["--no-newline", "--type", "image/png"])
					.output()
			} else {
				Command::new("xclip")
					.args(["-selection", "clipboard", "-t", "image/png", "-o"])
					.output()
			}
			.map_err(|_| {
				anyhow::anyhow!("Install wl-clipboard (Wayland) or xclip (X11) to paste pictures")
			})?;
			anyhow::ensure!(
				output.status.success(),
				"Clipboard has no PNG picture. Copy a picture and try again."
			);
			output.stdout
		}
		Source::Camera => {
			let output = Command::new("timeout")
				.args([
					"20s",
					"ffmpeg",
					"-hide_banner",
					"-loglevel",
					"error",
					"-f",
					"video4linux2",
					"-i",
					"/dev/video0",
					"-frames:v",
					"1",
					"-f",
					"image2pipe",
					"-vcodec",
					"png",
					"pipe:1",
				])
				.stdin(Stdio::null())
				.output()
				.map_err(|_| {
					anyhow::anyhow!("Install ffmpeg to take a picture with the default camera")
				})?;
			anyhow::ensure!(
				output.status.success(),
				"Cannot take picture. Check that the default camera /dev/video0 is connected and available."
			);
			output.stdout
		}
	};
	timeparagon_core::ai_input::image::extract(&bytes, settings).map(Some)
}

fn draft(f: Found, calendar_id: String, origin: Origin) -> Result<calendar::EditDraft, String> {
	use chrono::{NaiveDate, NaiveTime, TimeZone};
	let source = origin.noun();
	let parse_date = |s: &str| {
		NaiveDate::parse_from_str(s, "%Y-%m-%d")
			.map_err(|_| "The model returned an invalid date".to_string())
	};
	let parse_time = |s: &str| {
		NaiveTime::parse_from_str(s, "%H:%M")
			.map_err(|_| "The model returned an invalid time".to_string())
	};
	let day = f
		.date
		.as_deref()
		.map(parse_date)
		.transpose()?
		.unwrap_or(crate::clock::now().date_naive());
	let end_day = f
		.end_date
		.as_deref()
		.map(parse_date)
		.transpose()?
		.unwrap_or(day);
	let start_time = f.start_time.as_deref().map(parse_time).transpose()?;
	let end_time = f.end_time.as_deref().map(parse_time).transpose()?;
	let local = |d: NaiveDate, t: NaiveTime| {
		chrono::Local
			.from_local_datetime(&d.and_time(t))
			.single()
			.ok_or_else(|| {
				format!(
					"The {source} has an ambiguous or nonexistent local time; edit it and try again"
				)
			})
	};
	let all_day = start_time.is_none();
	let start = local(day, start_time.unwrap_or_default())?;
	let end = if all_day {
		local(end_day + chrono::Duration::days(1), NaiveTime::default())?
	} else if let Some(t) = end_time {
		local(end_day, t)?
	} else {
		start + chrono::Duration::hours(1)
	};
	if end <= start {
		return Err("The model returned an event ending before it starts".into());
	}
	let mut d = inbox::creation::picture_draft(calendar_id, start, end, all_day);
	d.title = f.title;
	d.description = f.description.unwrap_or_default();
	if let Some(location) = f.location.filter(|s| !s.is_empty()) {
		d.description.push_str(&format!("\nLocation: {location}"));
	}
	if f.date.is_none() {
		d.date_text.clear();
		d.description.push_str(&format!(
			"\nDate not found in {source}; enter a date before adding."
		));
	}
	if all_day {
		d.description.push_str(&format!(
			"\nNo start time found in {source}; review all-day setting."
		));
	}
	Ok(d)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	fn found(title: &str) -> Found {
		serde_json::from_value(json!({"title":title,"date":"2026-11-08","end_date":null,"start_time":"09:30","end_time":"10:45","description":"Bring notes","location":"Room 2"})).unwrap()
	}
	fn state() -> State {
		let mut s = State::blank();
		let _ = inbox::update(&mut s, inbox::Action::NewTask);
		s.inbox.image.events = vec![
			draft(found("One"), "cal".into(), Origin::Picture).unwrap(),
			draft(found("Two"), "cal".into(), Origin::Picture).unwrap(),
		];
		select(&mut s, 0);
		s
	}
	#[test]
	fn extracts_times_notes_and_multiday_all_day_dates() {
		let d = draft(found("Meeting"), "cal".into(), Origin::Picture).unwrap();
		assert_eq!(d.start_text, "09:30");
		assert_eq!(d.end_text, "10:45");
		assert!(d.description.contains("Room 2"));
		assert!(!d.all_day);
		let mut f = found("Festival");
		f.start_time = None;
		f.end_time = None;
		f.end_date = Some("2026-11-10".into());
		let d = draft(f, "cal".into(), Origin::Picture).unwrap();
		assert!(d.all_day);
		assert_eq!(d.end_date_text, "2026-11-10");
		assert_eq!(d.end.date_naive().to_string(), "2026-11-11");
	}
	#[test]
	fn missing_date_requires_manual_review_and_bad_dates_are_rejected() {
		let mut f = found("Unknown");
		f.date = None;
		assert!(
			draft(f, "cal".into(), Origin::Picture)
				.unwrap()
				.date_text
				.is_empty()
		);
		let mut f = found("Bad");
		f.date = Some("2026-02-30".into());
		assert!(draft(f, "cal".into(), Origin::Picture).is_err());
		let mut f = found("Bad");
		f.end_time = Some("08:00".into());
		assert!(draft(f, "cal".into(), Origin::Picture).is_err());
	}
	#[test]
	fn edits_survive_selection_and_removal() {
		let mut s = state();
		s.calendar.edit_draft.as_mut().unwrap().title = "Edited".into();
		let _ = update(&mut s, Action::Select(1));
		let _ = update(&mut s, Action::Select(0));
		assert_eq!(s.calendar.edit_draft.as_ref().unwrap().title, "Edited");
		let _ = update(&mut s, Action::Remove(0));
		assert_eq!(s.inbox.image.events.len(), 1);
		assert_eq!(s.calendar.edit_draft.as_ref().unwrap().title, "Two");
		let _ = update(&mut s, Action::Remove(0));
		assert!(s.inbox.image.events.is_empty());
		assert!(s.calendar.edit_draft.as_ref().unwrap().title.is_empty());
	}
	#[test]
	fn saving_one_keeps_other_proposals() {
		let mut s = state();
		s.inbox.draft.as_mut().unwrap().saving = true;
		assert!(saved(&mut s));
		assert_eq!(s.calendar.edit_draft.as_ref().unwrap().title, "Two");
		assert!(!s.inbox.draft.as_ref().unwrap().saving);
		assert!(!saved(&mut s));
	}
	#[test]
	fn stale_results_do_not_repopulate_a_closed_form() {
		let mut s = State::blank();
		let _ = update(&mut s, Action::Loaded(99, Ok(Some(vec![found("Old")]))));
		assert!(s.inbox.image.events.is_empty());
	}
	#[test]
	fn image_button_matches_the_mic_and_opens_sources() {
		use iced::{
			Size,
			widget::{Id, text_input},
		};
		use iced_test::{Simulator, core::Settings};
		let mut s = State::blank();
		let _ = inbox::update(&mut s, inbox::Action::NewTask);
		for width in [180., 420.] {
			let mut ui = Simulator::with_size(
				Settings::default(),
				Size::new(width, 240.),
				crate::voice::input(&s, text_input("Title", "").into()),
			);
			let mic = ui.find(Id::from("voice-record")).unwrap().bounds();
			let words = ui.find(Id::from("text-import")).unwrap().bounds();
			let picture = ui.find(Id::from("image-import")).unwrap().bounds();
			assert!(words.x >= mic.x + mic.width);
			assert!(picture.x >= words.x + words.width);
			assert_eq!(mic.size(), words.size());
			assert_eq!(mic.size(), picture.size());
			assert!(picture.x + picture.width <= width);
			ui.click(Id::from("image-import")).unwrap();
			assert!(
				ui.into_messages()
					.any(|m| matches!(m, Message::Inbox(inbox::Action::Image(Action::Menu))))
			);
		}
		let _ = update(&mut s, Action::Menu);
		let mut ui = Simulator::with_size(Settings::default(), Size::new(420., 240.), sources(&s));
		assert!(ui.find("Take picture").is_ok());
		assert!(ui.find("Paste image").is_ok());
		assert!(ui.find("Choose image file…").is_ok());
	}
	#[test]
	fn text_is_read_into_proposals_and_the_box_closes() {
		use std::sync::Arc;
		let mut s = State::blank();
		let _ = inbox::update(&mut s, inbox::Action::NewTask);
		let _ = update(&mut s, Action::Menu);
		let _ = update(&mut s, Action::TextMenu);
		assert!(s.inbox.image.text_open && !s.inbox.image.menu);
		// Nothing to read yet.
		let _ = update(&mut s, Action::ImportText);
		assert!(!s.inbox.image.busy);
		let pasted = "Dentist Friday 9am\nTalk at the library, date to be confirmed";
		let _ = update(
			&mut s,
			Action::TextEdit(text_editor::Action::Edit(text_editor::Edit::Paste(
				Arc::new(pasted.into()),
			))),
		);
		let _ = update(&mut s, Action::ImportText);
		assert!(s.inbox.image.busy);
		assert_eq!(s.inbox.image.reading, Origin::Text);
		// The form is locked while the text is read.
		let _ = update(&mut s, Action::TextMenu);
		assert!(s.inbox.image.text_open);
		let id = s.inbox.image.generation;
		let _ = update(&mut s, Action::Loaded(id, Ok(Some(vec![]))));
		assert!(s.inbox.image.error.as_deref().unwrap().contains("text"));
		assert_eq!(s.inbox.image.text.text().trim(), pasted);
		let _ = update(&mut s, Action::ImportText);
		let mut talk = found("Talk");
		talk.date = None;
		let id = s.inbox.image.generation;
		let _ = update(
			&mut s,
			Action::Loaded(id, Ok(Some(vec![found("Dentist"), talk]))),
		);
		assert!(!s.inbox.image.busy && s.inbox.image.error.is_none());
		assert!(!s.inbox.image.text_open);
		assert!(s.inbox.image.text.text().trim().is_empty());
		assert_eq!(s.inbox.image.events.len(), 2);
		assert_eq!(s.calendar.edit_draft.as_ref().unwrap().title, "Dentist");
		assert!(
			s.inbox.image.events[1]
				.description
				.contains("Date not found in text")
		);
	}
	#[test]
	fn text_box_offers_find_events_once_there_is_text() {
		use iced::Size;
		use iced_test::{Simulator, core::Settings};
		let mut s = State::blank();
		let _ = inbox::update(&mut s, inbox::Action::NewTask);
		let _ = update(&mut s, Action::TextMenu);
		let mut ui = Simulator::with_size(Settings::default(), Size::new(420., 300.), sources(&s));
		ui.click("Find events").unwrap();
		assert!(ui.into_messages().next().is_none());
		s.inbox.image.text = text_editor::Content::with_text("Lunch with Sam tomorrow at noon");
		let mut ui = Simulator::with_size(Settings::default(), Size::new(420., 300.), sources(&s));
		ui.click("Find events").unwrap();
		assert!(
			ui.into_messages()
				.any(|m| matches!(m, Message::Inbox(inbox::Action::Image(Action::ImportText))))
		);
	}
}
