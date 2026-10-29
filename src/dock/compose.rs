//! Interactive task card on the dock's layer surface.
use super::render::Layout;
use crate::inbox::Draft;
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers};

#[derive(Default)]
pub struct Compose {
    pub draft: Draft,
    pub focus: usize,
    cursor: usize,
    selected: bool,
}

pub fn contains((x, y, w, h): (f32, f32, f32, f32), px: f64, py: f64) -> bool {
    px >= x as f64 && px < (x + w) as f64 && py >= y as f64 && py < (y + h) as f64
}

pub fn rect(layout: &Layout) -> (f32, f32, f32, f32) {
    Layout {
        card_size: layout.card_size.max(300.0),
        ..*layout
    }
    .card_rect(f32::MAX, 300.0)
}

pub fn field_rect(layout: &Layout, index: usize) -> (f32, f32, f32, f32) {
    let (x, y, w, _) = rect(layout);
    (x + 12.0, y + 56.0 + index as f32 * 56.0, w - 24.0, 30.0)
}

pub fn action_rect(layout: &Layout, index: usize) -> (f32, f32, f32, f32) {
    let (x, y, w, _) = rect(layout);
    let width = (w - 32.0) / 2.0;
    (
        x + 12.0 + index as f32 * (width + 8.0),
        y + 218.0,
        width,
        32.0,
    )
}

impl Compose {
    fn value(&self, index: usize) -> &str {
        match index {
            0 => &self.draft.title,
            1 => &self.draft.deadline,
            _ => &self.draft.due,
        }
    }

    fn value_mut(&mut self) -> &mut String {
        match self.focus {
            0 => &mut self.draft.title,
            1 => &mut self.draft.deadline,
            _ => &mut self.draft.due,
        }
    }

    pub fn focus(&mut self, index: usize) {
        self.focus = index;
        self.cursor = self.value(index).len();
        self.selected = false;
    }

    pub fn insert(&mut self, text: &str) {
        if self.draft.saving {
            return;
        }
        if self.selected {
            self.value_mut().clear();
            self.cursor = 0;
            self.selected = false;
        }
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(1024)
            .collect();
        let cursor = self.cursor;
        self.value_mut().insert_str(cursor, &text);
        self.cursor += text.len();
        self.draft.error = None;
    }

    /// Single-line editor; submission and cancellation belong to the surface.
    pub fn key(&mut self, event: &KeyEvent, modifiers: Modifiers) {
        if self.draft.saving {
            return;
        }
        let value = self.value(self.focus);
        let previous = value[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(i, _)| i);
        let next = self.cursor
            + value[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        if modifiers.ctrl && event.keysym == Keysym::a {
            self.selected = true;
            return;
        }
        match event.keysym {
            Keysym::Tab | Keysym::ISO_Left_Tab => {
                self.focus((self.focus + if modifiers.shift { 2 } else { 1 }) % 3)
            }
            Keysym::Left => {
                self.cursor = previous;
                self.selected = false;
            }
            Keysym::Right => {
                self.cursor = next;
                self.selected = false;
            }
            Keysym::Home => {
                self.cursor = 0;
                self.selected = false;
            }
            Keysym::End => {
                self.cursor = value.len();
                self.selected = false;
            }
            Keysym::BackSpace | Keysym::Delete if self.selected => {
                self.value_mut().clear();
                self.cursor = 0;
                self.selected = false;
            }
            Keysym::BackSpace => {
                let cursor = self.cursor;
                self.value_mut().replace_range(previous..cursor, "");
                self.cursor = previous;
            }
            Keysym::Delete => {
                let cursor = self.cursor;
                self.value_mut().replace_range(cursor..next, "");
            }
            _ if !modifiers.ctrl && !modifiers.alt && !modifiers.logo => {
                if let Some(text) = &event.utf8 {
                    self.insert(text);
                }
            }
            _ => {}
        }
    }

    pub fn display(&self, index: usize, max_chars: usize) -> String {
        let value = self.value(index);
        let mut text = value.to_owned();
        if self.focus == index && !self.draft.saving {
            text.insert(self.cursor, if self.selected { '▏' } else { '│' });
        }
        let chars: Vec<char> = text.chars().collect();
        let cursor = if self.focus == index {
            value[..self.cursor].chars().count()
        } else {
            0
        };
        let start = cursor.saturating_sub(max_chars.saturating_sub(1));
        chars.into_iter().skip(start).take(max_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_unicode_and_changing_fields_keeps_byte_boundaries() {
        let mut form = Compose::default();
        form.insert("完成 task");
        form.focus(1);
        form.insert("2026-11-02");
        form.focus(0);
        form.insert(" today");
        assert_eq!(form.draft.title, "完成 task today");
        assert_eq!(form.draft.deadline, "2026-11-02");
        assert!(crate::inbox::validate(&form.draft).is_ok());
    }
    #[test]
    fn keyboard_edits_unicode_and_replaces_a_selected_title() {
        let mut form = Compose::default();
        let key = |keysym| KeyEvent {
            time: 0,
            raw_code: 0,
            keysym,
            utf8: None,
        };
        let modifiers = Modifiers::default();
        form.insert("a完成");
        form.key(&key(Keysym::Left), modifiers);
        form.key(&key(Keysym::BackSpace), modifiers);
        assert_eq!(form.draft.title, "a成");
        form.key(&key(Keysym::Delete), modifiers);
        assert_eq!(form.draft.title, "a");
        form.key(
            &key(Keysym::a),
            Modifiers {
                ctrl: true,
                ..modifiers
            },
        );
        form.insert("replacement");
        assert_eq!(form.draft.title, "replacement");
        form.key(&key(Keysym::Tab), modifiers);
        assert_eq!(form.focus, 1);
        form.key(
            &key(Keysym::ISO_Left_Tab),
            Modifiers {
                shift: true,
                ..modifiers
            },
        );
        assert_eq!(form.focus, 0);
    }

    #[test]
    fn saving_locks_the_form() {
        let mut form = Compose::default();
        form.insert("Task");
        form.draft.saving = true;
        form.insert(" extra");
        assert_eq!(form.draft.title, "Task");
    }
}
