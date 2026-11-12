//! The notepad page.

use super::*;

pub(super) fn update(state: &mut State, message: NotepadMessage) -> Task<Message> {
	match message {
		NotepadMessage::NotepadEdited(action) => {
			state.notepad.perform(action);

			// The editor is the one holding the text while it is being typed,
			// so the saved copy is taken from it after every change.
			state.config.notepad.text = state.notepad.text();

			queue_save(state)
		}
		NotepadMessage::NotepadFontSizeChanged(delta) => {
			let size = state.config.notepad.font_size;
			let next = size.saturating_add_signed(delta as i8).clamp(
				config::Notepad::MIN_FONT_SIZE,
				config::Notepad::MAX_FONT_SIZE,
			);

			state.config.notepad.font_size = next;

			queue_save(state)
		}
		NotepadMessage::NotepadTab(tab) => {
			state.notepad_tab = tab;
			if tab == crate::notes::Tab::Edit {
				iced::widget::operation::focus(crate::ui::notepad::input_id())
			} else {
				Task::none()
			}
		}
	}
}
