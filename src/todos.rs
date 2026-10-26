//! Temporary, client-side todos.
//!
//! A scratchpad for breaking a stretch of work into steps. It lives only in the
//! config file and never touches Google, so it is free to be thrown away and
//! rewritten as the day changes. The list is flat; the indent is only how an
//! item is drawn, which is what lets a few of them read as subtasks without the
//! bookkeeping a real tree would need.

use serde::{Deserialize, Serialize};

/// The deepest an item can be indented.
pub const MAX_INDENT: u8 = 3;

/// One scratchpad item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub text: String,
    /// How far the item is indented, from 0 to [`MAX_INDENT`]. Only the drawing
    /// uses it; the list underneath stays flat.
    #[serde(default)]
    pub indent: u8,
}

/// The temporary todo list, and which item is being worked on.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TodoList {
    #[serde(default)]
    items: Vec<Todo>,
    /// The item being worked on, as an index into `items`. Every mutation keeps
    /// it in range, so it is only ever read through [`TodoList::current`].
    #[serde(default)]
    current: usize,
}

impl TodoList {
    pub fn items(&self) -> &[Todo] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The index of the item being worked on, or `None` when the list is empty.
    pub fn current_index(&self) -> Option<usize> {
        (!self.items.is_empty()).then(|| self.current.min(self.items.len() - 1))
    }

    /// The item being worked on.
    pub fn current(&self) -> Option<&Todo> {
        self.current_index().and_then(|index| self.items.get(index))
    }

    /// A mutable handle to the item being worked on.
    pub fn current_mut(&mut self) -> Option<&mut Todo> {
        let index = self.current_index()?;
        self.items.get_mut(index)
    }

    /// Where the item being worked on sits, as `(position, total)` counting
    /// from one. This is the "2/4" the calendar shows.
    pub fn position(&self) -> Option<(usize, usize)> {
        self.current_index()
            .map(|index| (index + 1, self.items.len()))
    }

    /// Appends an item and returns its index. The new item becomes the one
    /// being worked on, so the thing just typed is what the clock shows.
    pub fn add(&mut self, text: impl Into<String>) -> usize {
        self.items.push(Todo {
            text: text.into(),
            indent: 0,
        });

        let index = self.items.len() - 1;
        self.current = index;
        index
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }

        self.items.remove(index);

        if self.items.is_empty() {
            self.current = 0;
        } else if index < self.current {
            // Everything after the hole slid up, so the item being worked on
            // moved with it.
            self.current -= 1;
        } else if self.current >= self.items.len() {
            // The last item was removed while it was the current one.
            self.current = self.items.len() - 1;
        }
    }

    pub fn set_text(&mut self, index: usize, text: impl Into<String>) {
        if let Some(todo) = self.items.get_mut(index) {
            todo.text = text.into();
        }
    }

    /// Moves an item up or down by `delta` places, carrying the "being worked
    /// on" pointer with it when it was the one that moved.
    pub fn move_item(&mut self, index: usize, delta: i32) {
        let len = self.items.len();
        if index >= len {
            return;
        }

        let target = index as i64 + i64::from(delta);
        if target < 0 || target >= len as i64 {
            return;
        }
        let target = target as usize;

        self.items.swap(index, target);

        if self.current == index {
            self.current = target;
        } else if self.current == target {
            self.current = index;
        }
    }

    /// Indents or outdents an item by `delta` levels, clamped to the range the
    /// drawing can show.
    pub fn indent(&mut self, index: usize, delta: i32) {
        if let Some(todo) = self.items.get_mut(index) {
            let level = i32::from(todo.indent) + delta;
            todo.indent = level.clamp(0, i32::from(MAX_INDENT)) as u8;
        }
    }

    pub fn set_current(&mut self, index: usize) {
        if index < self.items.len() {
            self.current = index;
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.current = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(texts: &[&str]) -> TodoList {
        let mut list = TodoList::default();
        for text in texts {
            list.add(*text);
        }
        list
    }

    #[test]
    fn the_item_just_added_becomes_the_one_being_worked_on() {
        let mut list = TodoList::default();
        assert_eq!(list.current(), None);

        list.add("write the report");
        assert_eq!(list.current().unwrap().text, "write the report");

        list.add("send the email");
        assert_eq!(list.current().unwrap().text, "send the email");
    }

    #[test]
    fn the_position_counts_from_one() {
        let mut list = list(&["a", "b", "c", "d"]);
        list.set_current(0);
        assert_eq!(list.position(), Some((1, 4)));

        list.set_current(1);
        assert_eq!(list.position(), Some((2, 4)));
    }

    #[test]
    fn removing_an_earlier_item_keeps_the_same_one_current() {
        let mut list = list(&["a", "b", "c"]);
        list.set_current(2);

        list.remove(0);

        assert_eq!(list.current().unwrap().text, "c");
        assert_eq!(list.position(), Some((2, 2)));
    }

    #[test]
    fn removing_the_current_item_moves_to_the_next_one() {
        let mut list = list(&["a", "b", "c"]);
        list.set_current(1);

        list.remove(1);

        assert_eq!(list.current().unwrap().text, "c");
    }

    #[test]
    fn removing_the_last_item_leaves_the_previous_one_current() {
        let mut list = list(&["a", "b"]);
        list.set_current(1);

        list.remove(1);

        assert_eq!(list.current().unwrap().text, "a");
    }

    #[test]
    fn emptying_the_list_leaves_nothing_current() {
        let mut list = list(&["only"]);
        list.remove(0);

        assert!(list.is_empty());
        assert_eq!(list.current(), None);
        assert_eq!(list.position(), None);
    }

    #[test]
    fn moving_an_item_carries_the_pointer_with_it() {
        let mut list = list(&["a", "b", "c"]);
        list.set_current(0);

        list.move_item(0, 1);

        assert_eq!(
            list.items()
                .iter()
                .map(|todo| todo.text.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert_eq!(list.current().unwrap().text, "a");
    }

    #[test]
    fn moving_an_item_past_the_end_does_nothing() {
        let mut list = list(&["a", "b"]);

        list.move_item(0, -1);
        list.move_item(1, 1);

        assert_eq!(
            list.items()
                .iter()
                .map(|todo| todo.text.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn indenting_stays_inside_the_range_the_drawing_shows() {
        let mut list = list(&["a"]);

        for _ in 0..10 {
            list.indent(0, 1);
        }
        assert_eq!(list.items()[0].indent, MAX_INDENT);

        for _ in 0..10 {
            list.indent(0, -1);
        }
        assert_eq!(list.items()[0].indent, 0);
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut list = list(&["a", "b"]);
        list.set_current(1);

        list.clear();

        assert!(list.is_empty());
        assert_eq!(list.current_index(), None);
    }
}
