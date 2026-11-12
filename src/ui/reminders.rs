//! The Reminders section of the settings page.

use iced::widget::{button, column, container, row, text};
use iced::{Center, Element, Fill};

use crate::app::{Message, State};
use crate::reminders::{Action, Category, Sound};
use timeparagon_core::reminders::{LEAD_CHOICES, lead_label};

use super::layout::Metrics;
use super::settings::{choice, indent, switch};

/// A lead time in the dropdown.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lead(u32);

impl std::fmt::Display for Lead {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let label = lead_label(self.0);
		if self.0 == 0 {
			f.write_str(&label)
		} else {
			write!(f, "{label} before")
		}
	}
}

fn message(action: Action) -> Message {
	Message::Reminders(action)
}

pub fn settings_section<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.reminders;
	let mut body = column![
		switch(
			"Remind me of what is coming up",
			settings.enabled,
			|enabled| message(Action::Toggled(enabled)),
			metrics,
		),
		text("Reminders are shown as desktop notifications. The sound plays at the alarm volume.")
			.size(metrics.label() - 1.0)
			.style(text::secondary),
	]
	.spacing(metrics.spacing);

	if !settings.enabled {
		return body.into();
	}

	let sounds = sound_choices(state);
	for category in Category::ALL {
		body = body.push(category_block(state, category, &sounds, metrics));
	}
	body.into()
}

fn sound_choices(state: &State) -> Vec<Sound> {
	let mut sounds = Sound::builtin_choices();
	if let Some(device) = &state.reminders.device_sounds {
		sounds.extend(device.iter().cloned());
	}
	// A saved sound from a theme that is no longer installed stays listed,
	// so the picker still shows what is chosen.
	for category in Category::ALL {
		let chosen = &state.config.reminders.get(category).sound;
		if !sounds.contains(chosen) {
			sounds.push(chosen.clone());
		}
	}
	sounds
}

fn category_block<'a>(
	state: &'a State,
	category: Category,
	sounds: &[Sound],
	metrics: &Metrics,
) -> Element<'a, Message> {
	let settings = state.config.reminders.get(category);
	let color = category.color(&state.config.calendar);
	let swatch = container(text(""))
		.width(12)
		.height(12)
		.style(move |_theme| container::Style {
			background: Some(color.into()),
			border: iced::Border {
				radius: 3.0.into(),
				..iced::Border::default()
			},
			..container::Style::default()
		});
	// Laid out like the calendar list's TimeParagon rows: swatch, name, switch.
	let header = row![
		swatch,
		text(category.label()).size(metrics.label()).width(Fill),
		crate::ui::controls::toggler(settings.enabled)
			.on_toggle(move |enabled| message(Action::CategoryToggled(category, enabled))),
	]
	.spacing(8)
	.align_y(Center);

	if !settings.enabled {
		return header.into();
	}

	let mut details = column![].spacing(metrics.spacing);
	if category == Category::Events {
		details = details.push(switch(
			"Use each event's Google Calendar reminders",
			state.config.reminders.use_calendar_reminders,
			|enabled| message(Action::CalendarRemindersToggled(enabled)),
			metrics,
		));
	}
	let lead_label =
		if category == Category::Events && state.config.reminders.use_calendar_reminders {
			"Otherwise remind"
		} else {
			category.lead_label()
		};
	details = details
		.push(choice(
			lead_label,
			LEAD_CHOICES.into_iter().map(Lead).collect(),
			Some(Lead(settings.minutes_before)),
			move |lead| message(Action::LeadChanged(category, lead.0)),
			metrics,
		))
		.push(choice(
			"Sound",
			sounds.to_vec(),
			Some(settings.sound.clone()),
			move |sound| message(Action::SoundChanged(category, sound)),
			metrics,
		))
		.push(
			button(text("Test").size(metrics.label()))
				.style(crate::ui::controls::secondary)
				.on_press(message(Action::Test(category))),
		);

	column![header, indent(details.into(), metrics)]
		.spacing(metrics.spacing)
		.into()
}
