//! The Notes screen and the section that shows a thing's notes in its details.
use iced::font::{Style as FontStyle, Weight};
use iced::widget::{
	Space, button, column, container, responsive, rich_text, row, span, text, text_editor,
};
use iced::{Border, Center, Color, Element, Fill, Font, Length, Theme};
use timeparagon_core::notes::markdown::{self, Block, Inline, Marker};
use timeparagon_core::notes::{Link, Note, Pastel};

use super::layout::Metrics;
use crate::app::{Message, State};
use crate::notes::{Action, Tab};

/// Text on a note. Every background is a light pastel, on any theme.
const INK: Color = Color::from_rgb8(0x20, 0x21, 0x24);
const FADED_INK: Color = Color { a: 0.65, ..INK };
const PREVIEW_CHARS: usize = 160;
const SWATCH: f32 = 20.0;

fn msg(action: Action) -> Message {
	Message::Notes(action)
}

fn pastel(color: Pastel) -> Color {
	let (r, g, b) = color.rgb();
	Color::from_rgb8(r, g, b)
}

pub(crate) fn note_surface(color: Pastel) -> impl Fn(&Theme) -> container::Style {
	move |_| container::Style {
		background: Some(pastel(color).into()),
		text_color: Some(INK),
		border: Border {
			color: Color { a: 0.12, ..INK },
			width: 1.0,
			radius: crate::ui::controls::RADIUS_ROW.into(),
		},
		..Default::default()
	}
}

/// A button that sits on a note, so it keeps to the note's colors rather than
/// the theme's.
fn on_note(_: &Theme, status: button::Status) -> button::Style {
	button::Style {
		background: matches!(status, button::Status::Hovered | button::Status::Pressed)
			.then(|| Color { a: 0.08, ..INK }.into()),
		text_color: INK,
		border: Border {
			radius: crate::ui::controls::RADIUS_ROW.into(),
			..Default::default()
		},
		..Default::default()
	}
}

fn swatches<'a>(id: &str, current: Pastel) -> Element<'a, Message> {
	let mut choices = row![].spacing(4.0);
	for color in Pastel::ALL {
		let selected = color == current;
		choices = choices.push(
			button(Space::new())
				.width(SWATCH)
				.height(SWATCH)
				.padding(0)
				.on_press(msg(Action::ColorChosen(id.to_owned(), color)))
				.style(move |_, _| button::Style {
					background: Some(pastel(color).into()),
					border: Border {
						color: if selected {
							INK
						} else {
							Color { a: 0.25, ..INK }
						},
						width: if selected { 2.0 } else { 1.0 },
						radius: (SWATCH / 2.0).into(),
					},
					..Default::default()
				}),
		);
	}
	choices.wrap().vertical_spacing(4.0).into()
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	match state.notes.open_note() {
		Some(note) => editor_page(state, note, metrics),
		None => list_page(state, metrics),
	}
}

// --------------------------------------------------------------------- list

fn list_page<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let store = &state.notes;
	let notes = store.library.search(&store.query);
	let size = metrics.label();

	let mut refresh = crate::ui::controls::quiet_button(text("Refresh").size(size));
	if !store.saving() {
		refresh = refresh.on_press(msg(Action::Refresh));
	}
	let actions = row![
		button(text("New note").size(size)).on_press(msg(Action::New)),
		refresh,
	]
	.spacing(metrics.spacing)
	.wrap()
	.vertical_spacing(metrics.spacing);

	let mut body = column![
		super::controls::page_heading(
			"Notes",
			"Markdown notes synced across your devices",
			Some(notes.len()),
			metrics,
		),
		actions,
		crate::ui::controls::text_input("Search notes…", &store.query)
			.on_input(|query| msg(Action::QueryChanged(query)))
			.size(size),
	]
	.spacing(metrics.spacing);
	if let Some(error) = &store.error {
		body = body.push(text(error.clone()).size(size).style(text::danger));
	}

	let metrics = *metrics;
	body.push(
		responsive(move |space| {
			if notes.is_empty() {
				let empty = if store.library.notes.is_empty() {
					"No notes yet."
				} else {
					"No matching notes."
				};
				return text(empty).size(metrics.label()).into();
			}
			// Each column stacks by itself, so a short note leaves no gap.
			let count = column_count(space.width, metrics.spacing);
			let mut columns: Vec<_> = (0..count)
				.map(|_| column![].spacing(metrics.spacing).width(Fill))
				.collect();
			for (index, note) in notes.iter().enumerate() {
				let slot = index % count;
				columns[slot] = std::mem::replace(&mut columns[slot], column![])
					.push(card(state, note, &metrics));
			}
			let cards = row(columns.into_iter().map(Element::from))
				.spacing(metrics.spacing)
				.padding(iced::Padding::default().right(12))
				.width(Fill);
			crate::ui::controls::scrollable(cards).height(Fill).into()
		})
		.height(Fill),
	)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.into()
}

fn column_count(width: f32, spacing: f32) -> usize {
	const MIN_CARD_WIDTH: f32 = 240.0;
	(((width - 12.0 + spacing) / (MIN_CARD_WIDTH + spacing)) as usize).clamp(1, 3)
}

fn card<'a>(state: &'a State, note: &'a Note, metrics: &Metrics) -> Element<'a, Message> {
	let size = metrics.label();
	let mut summary = column![
		text(note.display_title())
			.size(size + 3.0)
			.font(Font {
				weight: Weight::Bold,
				..Font::DEFAULT
			})
			.width(Fill)
	]
	.spacing(4.0);
	let preview = markdown::plain(&note.body, PREVIEW_CHARS);
	if !preview.is_empty() {
		summary = summary.push(text(preview).size(size).color(FADED_INK));
	}
	let open = button(summary)
		.width(Fill)
		.padding(4.0)
		.on_press(msg(Action::Open(note.id.clone())))
		.style(on_note);

	let mut content = column![open].spacing(4.0).width(Fill);
	for link in &note.links {
		content = content.push(
			container(
				text(format!(
					"{}: {}",
					link.kind.label(),
					crate::notes::current_title(state, link)
				))
				.size(size - 1.0)
				.color(FADED_INK),
			)
			.padding([0.0, 4.0]),
		);
	}
	content = content.push(
		button(text("Color").size(size - 1.0).color(FADED_INK))
			.padding([2.0, 4.0])
			.on_press(msg(Action::ColorMenuToggled(note.id.clone())))
			.style(on_note),
	);
	if state.notes.color_menu.as_deref() == Some(note.id.as_str()) {
		content = content.push(container(swatches(&note.id, note.color)).padding([0.0, 4.0]));
	}
	container(content)
		.padding(8.0)
		.width(Fill)
		.style(note_surface(note.color))
		.into()
}

// ------------------------------------------------------------------- editor

fn editor_page<'a>(state: &'a State, note: &'a Note, metrics: &Metrics) -> Element<'a, Message> {
	let store = &state.notes;
	let size = metrics.label();
	let tab = |label: &'static str, which: Tab| {
		button(text(label).size(size))
			.on_press(msg(Action::ShowTab(which)))
			.style(crate::ui::controls::nav(store.tab == which))
	};
	let top = row![
		crate::ui::controls::quiet_button(text("Back").size(size)).on_press(msg(Action::Back)),
		Space::new().width(Fill),
		tab("Edit", Tab::Edit),
		tab("View", Tab::View),
	]
	.spacing(metrics.spacing)
	.align_y(Center);

	let title = crate::ui::controls::text_input(notes_untitled(), &note.title)
		.id(crate::notes::title_input_id())
		.on_input(|title| msg(Action::TitleChanged(title)))
		.size(size + 4.0);

	let color = note.color;
	let page: Element<'a, Message> = match store.tab {
		Tab::Edit => crate::ui::controls::text_editor(&store.editor)
			.placeholder("Write in Markdown: # headings, - bullets, **bold**, *italics*")
			.on_action(|action| msg(Action::Edited(action)))
			.size(size + 1.0)
			.height(Fill)
			.style(move |theme, status| text_editor::Style {
				background: pastel(color).into(),
				placeholder: FADED_INK,
				value: INK,
				..crate::ui::controls::editor_style(theme, status)
			})
			.into(),
		Tab::View => container(
			crate::ui::controls::scrollable(container(rendered(&note.body, size + 1.0)).padding(
				iced::Padding {
					right: 14.0,
					..iced::Padding::ZERO
				},
			))
			.width(Fill)
			.height(Fill),
		)
		.padding(10.0)
		.width(Fill)
		.height(Fill)
		.style(note_surface(color))
		.into(),
	};

	let mut footer = column![
		crate::ui::controls::quiet_button(text("Open in default app").size(size))
			.on_press(msg(Action::OpenExternal)),
		swatches(&note.id, note.color),
		connections(state, note, metrics),
	]
	.spacing(metrics.spacing)
	.width(Fill);
	if let Some(error) = &store.error {
		footer = footer.push(text(error.clone()).size(size).style(text::danger));
	}
	footer = footer.push(if store.confirm_delete {
		Element::from(
			column![
				text("Delete this note from all devices?").size(size),
				row![
					crate::ui::controls::quiet_button(text("Cancel").size(size))
						.on_press(msg(Action::DeleteCancelled)),
					button(text("Delete").size(size))
						.on_press(msg(Action::DeleteConfirmed))
						.style(crate::ui::controls::danger),
				]
				.spacing(metrics.spacing),
			]
			.spacing(4.0),
		)
	} else {
		crate::ui::controls::quiet_button(text("Delete note").size(size))
			.on_press(msg(Action::DeleteRequested))
			.into()
	});

	// The text gets the room; what is below scrolls in a part of the height.
	let footer_height = (metrics.size.height * 0.35).max(80.0);
	column![
		top,
		title,
		page,
		container(
			crate::ui::controls::scrollable(container(footer).padding(iced::Padding {
				right: 14.0,
				..iced::Padding::ZERO
			}))
			.width(Fill)
			.height(Length::Shrink)
		)
		.max_height(footer_height),
	]
	.spacing(metrics.spacing)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.into()
}

fn notes_untitled() -> &'static str {
	timeparagon_core::notes::UNTITLED
}

/// What the open note is connected to, and a search for more.
fn connections<'a>(state: &'a State, note: &'a Note, metrics: &Metrics) -> Element<'a, Message> {
	let size = metrics.label();
	let mut list = column![text("Connected to").size(size).style(text::secondary)]
		.spacing(4.0)
		.width(Fill);
	for link in &note.links {
		list = list.push(
			row![
				button(text(link_label(state, link)).size(size).width(Fill))
					.width(Fill)
					.padding([2.0, 4.0])
					.on_press(msg(Action::Follow(link.clone())))
					.style(crate::ui::controls::ghost),
				button(text("×").size(size))
					.padding([2.0, 6.0])
					.on_press(msg(Action::Disconnect(note.id.clone(), link.clone())))
					.style(crate::ui::controls::ghost),
			]
			.align_y(Center),
		);
	}
	list = list.push(
		crate::ui::controls::text_input(
			"Find an event, time block, task or habit…",
			&state.notes.connect_query,
		)
		.on_input(|query| msg(Action::ConnectQueryChanged(query)))
		.size(size),
	);
	let found = crate::notes::candidates(state);
	if found.is_empty() && !state.notes.connect_query.trim().is_empty() {
		list = list.push(
			text("Nothing matches among the habits and the days loaded around this one.")
				.size(size)
				.style(text::secondary),
		);
	}
	for link in found {
		list = list.push(
			button(
				text(format!("{}: {}", link.kind.label(), link.title))
					.size(size)
					.width(Fill),
			)
			.width(Fill)
			.padding([2.0, 4.0])
			.on_press(msg(Action::Connect(note.id.clone(), link)))
			.style(crate::ui::controls::secondary),
		);
	}
	list.into()
}

fn link_label(state: &State, link: &Link) -> String {
	let mut label = format!(
		"{}: {}",
		link.kind.label(),
		crate::notes::current_title(state, link)
	);
	if let Some(day) = link.day {
		label.push_str(&day.format(" · %b %-d").to_string());
	}
	label
}

// ----------------------------------------------------------------- markdown

fn styled<'a>(runs: Vec<Inline>, size: f32, bold: bool) -> Element<'a, Message> {
	let spans: Vec<_> = runs
		.into_iter()
		.map(|run| {
			let font = if run.code {
				Font::MONOSPACE
			} else {
				Font {
					weight: if run.bold || bold {
						Weight::Bold
					} else {
						Weight::Normal
					},
					style: if run.italic {
						FontStyle::Italic
					} else {
						FontStyle::Normal
					},
					..Font::DEFAULT
				}
			};
			span(run.text)
				.font(font)
				.color(INK)
				.underline(run.link.is_some())
				.strikethrough(run.strike)
				.link_maybe(run.link)
		})
		.collect();
	rich_text(spans)
		.size(size)
		.width(Fill)
		.on_link_click(Message::OpenEventLink)
		.into()
}

/// A note's text as it reads, on a note's background.
pub(crate) fn rendered<'a>(body: &str, size: f32) -> Element<'a, Message> {
	let blocks = markdown::parse(body);
	if blocks.is_empty() {
		return text("Nothing written yet. Switch to Edit to start.")
			.size(size)
			.color(FADED_INK)
			.into();
	}
	let mut page = column![].spacing(size * 0.6).width(Fill);
	for block in blocks {
		page = page.push(match block {
			Block::Heading { level, text } => {
				let scale = match level {
					1 => 1.6,
					2 => 1.35,
					3 => 1.15,
					_ => 1.0,
				};
				styled(text, size * scale, true)
			}
			Block::Paragraph(text) => styled(text, size, false),
			Block::Item {
				depth,
				marker,
				text: runs,
			} => {
				let glyph = match marker {
					Marker::Bullet => "•".to_owned(),
					Marker::Number(number) => format!("{number}."),
					Marker::Check(true) => "☑".to_owned(),
					Marker::Check(false) => "☐".to_owned(),
					Marker::Continued => String::new(),
				};
				row![
					Space::new().width(depth as f32 * size * 1.2),
					container(text(glyph).size(size).color(INK)).width(size * 1.5),
					styled(runs, size, false),
				]
				.into()
			}
			Block::Quote(runs) => row![
				container(Space::new())
					.width(3.0)
					.height(size * 1.3)
					.style(|_| container::Style {
						background: Some(FADED_INK.into()),
						..Default::default()
					}),
				styled(runs, size, false),
			]
			.spacing(8.0)
			.into(),
			Block::Code(code) => container(text(code).size(size).font(Font::MONOSPACE).color(INK))
				.padding(6.0)
				.width(Fill)
				.style(|_| container::Style {
					background: Some(Color { a: 0.07, ..INK }.into()),
					border: Border {
						radius: crate::ui::controls::RADIUS_SMALL.into(),
						..Default::default()
					},
					..Default::default()
				})
				.into(),
			Block::Rule => container(Space::new())
				.width(Fill)
				.height(1.0)
				.style(|_| container::Style {
					background: Some(Color { a: 0.3, ..INK }.into()),
					..Default::default()
				})
				.into(),
		});
	}
	page.into()
}

// ------------------------------------------------------------------ section

#[derive(Debug, Clone, PartialEq)]
struct NoteChoice {
	id: String,
	title: String,
}

impl std::fmt::Display for NoteChoice {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.title)
	}
}

/// The notes connected to something, for its details: an event's, a time
/// block's, a task's or a habit's.
pub fn section<'a>(state: &'a State, target: Link, metrics: &Metrics) -> Element<'a, Message> {
	let size = metrics.label();
	let library = &state.notes.library;
	let mut list = column![crate::ui::controls::field_label("Notes", size)]
		.spacing(4.0)
		.width(Fill);
	for note in library.linked(&target) {
		list = list.push(
			container(
				row![
					button(text(note.display_title()).size(size).width(Fill))
						.width(Fill)
						.padding([2.0, 4.0])
						.on_press(msg(Action::Open(note.id.clone())))
						.style(on_note),
					button(text("×").size(size))
						.padding([2.0, 6.0])
						.on_press(msg(Action::Disconnect(note.id.clone(), target.clone())))
						.style(on_note),
				]
				.align_y(Center),
			)
			.width(Fill)
			.style(note_surface(note.color)),
		);
	}
	let new_note = button(text("New note").size(size).center().width(Fill))
		.width(Fill)
		.on_press(msg(Action::NewFor(target.clone())))
		.style(crate::ui::controls::secondary);
	list = list.push(new_note);

	let others: Vec<NoteChoice> = library
		.search("")
		.into_iter()
		.filter(|note| !note.is_linked(&target))
		.map(|note| NoteChoice {
			id: note.id.clone(),
			title: note.display_title().to_owned(),
		})
		.collect();
	if !others.is_empty() {
		list = list.push(
			crate::ui::controls::pick_list(
				others,
				None::<NoteChoice>,
				move |choice: NoteChoice| msg(Action::Connect(choice.id, target.clone())),
			)
			.placeholder("Connect a note…")
			.text_size(size)
			.width(Fill),
		);
	}
	list.into()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::app::{Screen, State};
	use iced::Size;
	use iced_test::Simulator;
	use iced_test::core::Settings;
	use timeparagon_core::notes::LinkKind;

	fn simulator<'a>(
		view: Element<'a, Message>,
		size: Size,
	) -> Simulator<'a, Message, Theme, iced::Renderer> {
		Simulator::with_settings(
			Settings {
				..Settings::default()
			},
			container(view).width(size.width).height(size.height),
		)
	}

	fn state_with_note() -> (State, String) {
		let mut state = State::blank();
		state.screen = Screen::Notes;
		let id = state.notes.library.create("Reading list", Vec::new());
		state
			.notes
			.library
			.edit(&id, |note| note.body = "# Books\n\n- **Dune**\n".into());
		(state, id)
	}

	fn metrics(width: f32) -> Metrics {
		Metrics::of(Size::new(width, 700.0))
	}

	#[test]
	fn the_list_shows_each_note_and_its_preview() {
		let (state, id) = state_with_note();
		for width in [180.0, 300.0, 900.0] {
			let metrics = metrics(width);
			let mut ui = simulator(view(&state, &metrics), Size::new(width, 700.0));
			ui.find("Reading list").unwrap();
			ui.click("Reading list").unwrap();
			assert!(ui.into_messages().any(
				|message| matches!(&message, Message::Notes(Action::Open(open)) if *open == id)
			));
		}
	}

	#[test]
	fn the_color_button_offers_the_swatches() {
		let (mut state, id) = state_with_note();
		let metrics = metrics(300.0);
		let mut ui = simulator(view(&state, &metrics), Size::new(300.0, 700.0));
		ui.click("Color").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			&message,
			Message::Notes(Action::ColorMenuToggled(open)) if *open == id
		)));
		state.notes.color_menu = Some(id);
		let _ = view(&state, &metrics);
	}

	#[test]
	fn an_open_note_has_edit_and_view_tabs() {
		let (mut state, id) = state_with_note();
		state.notes.open = Some(id);
		state.notes.tab = Tab::View;
		for width in [180.0, 900.0] {
			let metrics = metrics(width);
			let mut ui = simulator(view(&state, &metrics), Size::new(width, 700.0));
			// Rich text has no selector, but a list item's marker does.
			ui.find("•").unwrap();
			ui.click("Edit").unwrap();
			assert!(
				ui.into_messages()
					.any(|message| matches!(message, Message::Notes(Action::ShowTab(Tab::Edit))))
			);
		}
	}

	#[test]
	fn deleting_a_note_says_what_it_will_do() {
		let (mut state, id) = state_with_note();
		state.notes.open = Some(id);
		let metrics = metrics(400.0);
		let mut ui = simulator(view(&state, &metrics), Size::new(400.0, 700.0));
		ui.find("Delete note").unwrap();
		drop(ui);
		state.notes.confirm_delete = true;
		let mut ui = simulator(view(&state, &metrics), Size::new(400.0, 700.0));
		ui.click("Delete").unwrap();
		assert!(
			ui.into_messages()
				.any(|message| matches!(message, Message::Notes(Action::DeleteConfirmed)))
		);
	}

	#[test]
	fn a_things_details_list_its_notes_and_can_add_one() {
		let (mut state, id) = state_with_note();
		let target = Link {
			kind: LinkKind::Task,
			id: "task:1".into(),
			title: "Task".into(),
			day: None,
		};
		let metrics = metrics(300.0);
		let mut ui = simulator(
			section(&state, target.clone(), &metrics),
			Size::new(300.0, 300.0),
		);
		assert!(ui.find("Reading list").is_err(), "not connected yet");
		ui.click("New note").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			&message,
			Message::Notes(Action::NewFor(link)) if link.same_target(&target)
		)));

		state.notes.library.connect(&id, target.clone());
		let mut ui = simulator(section(&state, target, &metrics), Size::new(300.0, 300.0));
		ui.click("Reading list").unwrap();
		assert!(
			ui.into_messages().any(
				|message| matches!(&message, Message::Notes(Action::Open(open)) if *open == id)
			)
		);
	}
}
